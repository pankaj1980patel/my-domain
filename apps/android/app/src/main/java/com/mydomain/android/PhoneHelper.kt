package com.mydomain.android

import android.accessibilityservice.AccessibilityService
import android.app.KeyguardManager
import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.provider.Settings
import android.util.Log
import android.view.accessibility.AccessibilityEvent
import android.view.accessibility.AccessibilityNodeInfo

/**
 * my-domain's Accessibility service: what lets your computers do on this
 * phone what you'd otherwise tap yourself, on any stock phone (no
 * Developer options, no root). You turn it on once in Settings →
 * Accessibility.
 *
 *  - Radios ([switchRadio]): open the switch's settings panel, find the
 *    switch, tap it, check the radio really changed, close the panel. If
 *    the switch can't be found (each maker's Settings looks different) the
 *    panel stays open for you, as without the service.
 *  - Keys ([press]): Back, Home, recent apps, notifications, Quick
 *    Settings, lock the screen.
 *  - Taps and swipes ([touch]): from a computer you share this screen
 *    with, only while you allow control in the sharing notification.
 *
 * It reads screens only while doing one of these, and only Settings'.
 */
class PhoneHelper : AccessibilityService() {
    private val main = Handler(Looper.getMainLooper())

    override fun onServiceConnected() {
        instance = this
        Log.i(TAG, "on")
        Controls.onChange()
    }

    override fun onDestroy() {
        if (instance === this) instance = null
        Controls.onChange()
        super.onDestroy()
    }

    override fun onAccessibilityEvent(event: AccessibilityEvent?) {}
    override fun onInterrupt() {}

    /** Open [panel], tap the switch for radio [what] so it's [on], check,
     *  close. [done] gets whether it worked (on the main thread). */
    fun switchRadio(what: String, on: Boolean, panel: Intent, done: (Boolean) -> Unit) {
        main.post {
            if (runCatching { startActivity(panel.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)) }.isFailure) {
                done(false)
                return@post
            }
            val start = System.currentTimeMillis()
            var tapped = false
            val step = object : Runnable {
                override fun run() {
                    val now = Radios.isOn(this@PhoneHelper, what)
                    if (now == on) {
                        // Changed (or already was): close the panel.
                        main.postDelayed({ performGlobalAction(GLOBAL_ACTION_BACK) }, CLOSE_DELAY_MS)
                        done(true)
                        return
                    }
                    if (System.currentTimeMillis() - start > TIMEOUT_MS) {
                        Log.w(TAG, "$what: switch not found or didn't change; left open")
                        done(false)
                        return
                    }
                    // Only ever in Settings: never tap anything in another app
                    // (before the panel opens, another window is in front).
                    val root = rootInActiveWindow?.takeIf { isSettings(it.packageName) }
                    if (!tapped && root != null) {
                        val sw = findSwitch(root, what)
                        if (sw != null && sw.isChecked != on) {
                            tapped = clickable(sw)?.performAction(AccessibilityNodeInfo.ACTION_CLICK) == true
                            Log.i(TAG, "$what: tapped=$tapped (${sw.className}, \"${sw.text ?: sw.contentDescription ?: ""}\")")
                        }
                    }
                    main.postDelayed(this, STEP_MS)
                }
            }
            main.postDelayed(step, FIRST_LOOK_MS)
        }
    }

    /** Press a phone key; false if [key] is unknown or it didn't work. */
    fun press(key: String): Boolean {
        val action = when (key) {
            "back" -> GLOBAL_ACTION_BACK
            "home" -> GLOBAL_ACTION_HOME
            "recents" -> GLOBAL_ACTION_RECENTS
            "notifications" -> GLOBAL_ACTION_NOTIFICATIONS
            "quick_settings" -> GLOBAL_ACTION_QUICK_SETTINGS
            "lock" -> if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.P) GLOBAL_ACTION_LOCK_SCREEN else return false
            else -> return false
        }
        return performGlobalAction(action)
    }

    /** Touch the screen at ([x], [y]) and move to ([x2], [y2]) over [ms]
     *  milliseconds, in screen pixels: a tap, a long press (held still) or
     *  a swipe. Only for a computer you share this screen with and allowed
     *  control (ScreenShare.onInput). Played on the main thread, in the
     *  order they came; false if it couldn't be built. */
    fun touch(x: Float, y: Float, x2: Float, y2: Float, ms: Long): Boolean {
        val gesture = runCatching {
            val path = android.graphics.Path().apply {
                moveTo(x, y)
                lineTo(x2, y2)
            }
            val stroke = android.accessibilityservice.GestureDescription.StrokeDescription(path, 0, ms.coerceIn(1, 3_000))
            android.accessibilityservice.GestureDescription.Builder().addStroke(stroke).build()
        }.onFailure { Log.w(TAG, "touch: $it") }.getOrNull() ?: return false
        main.post {
            if (!runCatching { dispatchGesture(gesture, null, null) }.getOrDefault(false)) Log.w(TAG, "touch not played")
        }
        return true
    }

    companion object {
        private const val TAG = "mydomain_helper"
        /** The panel's first draw. */
        private const val FIRST_LOOK_MS = 700L
        private const val STEP_MS = 300L
        private const val TIMEOUT_MS = 6_000L
        /** Long enough to see the switch move. */
        private const val CLOSE_DELAY_MS = 600L

        val KEYS = listOf("back", "home", "recents", "notifications", "quick_settings", "lock")

        @Volatile
        var instance: PhoneHelper? = null
            private set

        /** On, and able to act now (Settings can't open over the lock screen). */
        fun ready(ctx: Context): Boolean {
            if (instance == null) return false
            val km = ctx.getSystemService(KeyguardManager::class.java)
            return km?.isKeyguardLocked != true
        }

        /** Turned on in Settings → Accessibility (it may still be starting). */
        fun enabled(ctx: Context): Boolean {
            val me = ComponentName(ctx, PhoneHelper::class.java).flattenToString()
            val list = Settings.Secure.getString(ctx.contentResolver, Settings.Secure.ENABLED_ACCESSIBILITY_SERVICES) ?: return false
            return list.split(':').any { it.equals(me, ignoreCase = true) }
        }

        /** Words a radio's switch is labelled with (English; the panel's
         *  first switch is the fallback for Wi-Fi and Bluetooth). */
        private val LABELS = mapOf(
            "wifi" to listOf("wi-fi", "wifi", "wlan"),
            "bluetooth" to listOf("bluetooth"),
            "airplane" to listOf("airplane", "aeroplane", "flight"),
            "data" to listOf("mobile data", "cellular data", "mobile network"),
            "hotspot" to listOf("hotspot", "tethering"),
            "location" to listOf("location"),
        )

        /** The switch for [what] in this window: a checkable node labelled
         *  (itself or its row) with one of the radio's words. */
        fun findSwitch(root: AccessibilityNodeInfo, what: String): AccessibilityNodeInfo? {
            val words = LABELS[what] ?: return null
            val switches = mutableListOf<AccessibilityNodeInfo>()
            collect(root, switches)
            val says = { t: String -> words.any { it in t } }
            // Its own label first (vivo: the row's Switch is described
            // "Bluetooth"), then its row's, then the page's first switch.
            switches.firstOrNull { says(ownText(it)) }?.let { return it }
            switches.firstOrNull { says(rowText(it)) }?.let { return it }
            return if (what == "wifi" || what == "bluetooth" || what == "location") switches.firstOrNull() else null
        }

        /** Settings, or a maker's own Settings app. */
        fun isSettings(pkg: CharSequence?): Boolean = pkg?.toString()?.lowercase()?.contains("settings") == true

        private fun ownText(n: AccessibilityNodeInfo): String =
            "${n.text ?: ""} ${n.contentDescription ?: ""}".lowercase()

        private fun collect(n: AccessibilityNodeInfo, out: MutableList<AccessibilityNodeInfo>) {
            if (n.isCheckable && n.isEnabled) out += n
            for (i in 0 until n.childCount) n.getChild(i)?.let { collect(it, out) }
        }

        /** The text of a switch and of the row it sits in (two levels up). */
        private fun rowText(sw: AccessibilityNodeInfo): String {
            val row = sw.parent?.parent ?: sw.parent ?: sw
            val sb = StringBuilder()
            fun walk(n: AccessibilityNodeInfo, depth: Int) {
                n.text?.let { sb.append(it).append(' ') }
                n.contentDescription?.let { sb.append(it).append(' ') }
                if (depth < 4) for (i in 0 until n.childCount) n.getChild(i)?.let { walk(it, depth + 1) }
            }
            walk(row, 0)
            return sb.toString().lowercase()
        }

        /** The node to click: the switch, or the row that handles its taps. */
        private fun clickable(n: AccessibilityNodeInfo): AccessibilityNodeInfo? {
            var cur: AccessibilityNodeInfo? = n
            repeat(4) {
                if (cur?.isClickable == true) return cur
                cur = cur?.parent
            }
            return n
        }
    }
}
