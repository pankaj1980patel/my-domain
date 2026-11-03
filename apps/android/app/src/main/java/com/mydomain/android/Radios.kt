package com.mydomain.android

import android.Manifest
import android.bluetooth.BluetoothAdapter
import android.bluetooth.BluetoothManager
import android.content.BroadcastReceiver
import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.content.pm.PackageManager
import android.net.wifi.WifiManager
import android.os.Build
import android.provider.Settings
import android.util.Log
import org.json.JSONObject

/**
 * The phone's radios for your other devices (wave E, ADR-0006): airplane
 * mode, mobile data and hotspot in the status (Wi-Fi and Bluetooth are
 * there already), and the `radio {what, on}` action to switch one.
 *
 * Android doesn't let an ordinary app switch most radios: Wi-Fi only for
 * apps targeting Android 9 or older, Bluetooth only for those targeting 12
 * or older, airplane mode, mobile data and hotspot never. This app targets
 * 9 on purpose, so it switches Wi-Fi and Bluetooth directly, even locked.
 * The rest open the matching settings panel on the phone (a notification
 * to tap when the app isn't on screen). With
 * my-domain's Accessibility service on ([PhoneHelper]) and the phone
 * unlocked, the service taps the switch in that panel itself; with Shizuku
 * running and allowed ([Privileged]), all but the hotspot switch with no
 * screen at all.
 */
object Radios {
    private const val TAG = "mydomain_radios"
    val KINDS = listOf("wifi", "bluetooth", "airplane", "data", "hotspot")

    private var receiver: BroadcastReceiver? = null
    /** False once this phone refused to switch Bluetooth directly. */
    @Volatile
    private var btDirect = true

    /** Tell [Controls.onChange] whenever a radio is switched, by anyone. */
    fun init(ctx: Context) {
        Privileged.onChange = { Controls.onChange() }
        Privileged.init()
        if (receiver != null) return
        val r = object : BroadcastReceiver() {
            override fun onReceive(c: Context, i: Intent) = Controls.onChange()
        }
        val f = IntentFilter().apply {
            addAction(WifiManager.WIFI_STATE_CHANGED_ACTION)
            addAction(BluetoothAdapter.ACTION_STATE_CHANGED)
            addAction(Intent.ACTION_AIRPLANE_MODE_CHANGED)
            addAction(AP_STATE_CHANGED)
        }
        runCatching { ctx.applicationContext.registerReceiver(r, f) }
            .onSuccess { receiver = r }
            .onFailure { Log.w(TAG, "can't watch the radios: $it") }
    }

    /** Airplane mode, mobile data and hotspot, as far as they can be read. */
    fun state(ctx: Context, o: JSONObject) {
        val cr = ctx.contentResolver
        runCatching { o.put("airplane", Settings.Global.getInt(cr, Settings.Global.AIRPLANE_MODE_ON, 0) == 1) }
        if (ctx.packageManager.hasSystemFeature(PackageManager.FEATURE_TELEPHONY)) {
            // The user's switch, the same one Settings shows.
            runCatching { Settings.Global.getInt(cr, "mobile_data", -1) }.getOrNull()
                ?.takeIf { it >= 0 }?.let { o.put("mobile_data", it == 1) }
        }
        hotspotOn(ctx)?.let { o.put("hotspot", it) }
        // The ones that switch with no tap on the phone.
        val direct = when {
            PhoneHelper.instance != null -> KINDS
            Privileged.ready() -> KINDS.filter { Privileged.radioCommand(it, true) != null }
            // Targeting Android 9, Wi-Fi switches with nothing else.
            !Sdk.rules(Build.VERSION_CODES.Q) -> listOf("wifi")
            else -> emptyList()
        }
        if (direct.isNotEmpty()) o.put("radio_direct", org.json.JSONArray(direct))
    }

    /** Whether radio [what] is on now; null if it can't be read. */
    fun isOn(ctx: Context, what: String): Boolean? = runCatching {
        val app = ctx.applicationContext
        when (what) {
            "wifi" -> app.getSystemService(WifiManager::class.java)?.isWifiEnabled
            "bluetooth" -> app.getSystemService(BluetoothManager::class.java)?.adapter?.isEnabled
            "airplane" -> Settings.Global.getInt(app.contentResolver, Settings.Global.AIRPLANE_MODE_ON, 0) == 1
            "data" -> Settings.Global.getInt(app.contentResolver, "mobile_data", -1).takeIf { it >= 0 }?.let { it == 1 }
            "hotspot" -> hotspotOn(app)
            else -> null
        }
    }.getOrNull()

    /** Hotspot on or off: a hidden but long-standing WifiManager call. */
    private fun hotspotOn(ctx: Context): Boolean? = runCatching {
        val wm = ctx.applicationContext.getSystemService(WifiManager::class.java) ?: return null
        wm.javaClass.getMethod("isWifiApEnabled").invoke(wm) as? Boolean
    }.getOrNull()

    /** Switch radio [what] to [on]: directly where Android allows it, else
     *  by opening its settings for you to tap. False if [what] is unknown. */
    fun set(ctx: Context, fromName: String, what: String, on: Boolean): Boolean {
        if (what !in KINDS) return false
        val app = ctx.applicationContext
        Log.i(TAG, "$fromName: $what ${if (on) "on" else "off"}")
        Privileged.radioCommand(what, on)?.let { cmd -> if (Privileged.run(*cmd)) return true }
        if (direct(app, what, on)) return true
        val intent = panel(app, what) ?: return false
        PhoneHelper.instance?.takeIf { PhoneHelper.ready(app) }?.let { helper ->
            // Taps the switch in the panel; if it can't, the panel stays
            // open on the phone, as below.
            helper.switchRadio(what, on, intent) { ok -> Log.i(TAG, "$what by Accessibility: $ok") }
            return true
        }
        if (what == "bluetooth" && on && bluetoothPrompt(app)) return true
        val label = app.getString(
            when (what) {
                "wifi" -> R.string.radio_wifi
                "bluetooth" -> R.string.radio_bluetooth
                "airplane" -> R.string.radio_airplane
                "data" -> R.string.radio_data
                else -> R.string.radio_hotspot
            },
        )
        val title = app.getString(if (on) R.string.radio_turn_on_title else R.string.radio_turn_off_title, fromName, label)
        DeviceActions.startOrNotify(app, intent, title, app.getString(R.string.radio_body))
        return true
    }

    /** Wi-Fi and Bluetooth the old way, on the Android versions that allow it. */
    @Suppress("DEPRECATION")
    private fun direct(app: Context, what: String, on: Boolean): Boolean = runCatching {
        when (what) {
            // Blocked only for apps targeting Android 10+ (this one targets 9).
            "wifi" -> (!Sdk.rules(Build.VERSION_CODES.Q) &&
                app.getSystemService(WifiManager::class.java)?.setWifiEnabled(on) == true)
                .also { Log.i(TAG, "wifi directly: $it") }
            "bluetooth" -> {
                val bt = app.getSystemService(BluetoothManager::class.java)?.adapter ?: return false
                val allowed = !Sdk.rules(Build.VERSION_CODES.S) ||
                    app.checkSelfPermission(Manifest.permission.BLUETOOTH_CONNECT) == PackageManager.PERMISSION_GRANTED
                // Blocked only for apps targeting Android 13+.
                // Some phones refuse even old-target apps (vivo, Android 16),
                // after a 3 s wait: once refused, go straight to the helper.
                (btDirect && allowed && !Sdk.rules(Build.VERSION_CODES.TIRAMISU) && if (on) bt.enable() else bt.disable())
                    .also { if (!it && allowed) btDirect = false; Log.i(TAG, "bluetooth directly: $it") }
            }
            else -> false
        }
    }.onFailure { Log.w(TAG, "can't switch $what: $it") }.getOrDefault(false)

    /** "Allow my-domain to turn on Bluetooth?" (Android 13+): one tap, and
     *  it needs the screen like any other screen. */
    private fun bluetoothPrompt(app: Context): Boolean {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.TIRAMISU) return false
        if (Sdk.rules(Build.VERSION_CODES.S) &&
            app.checkSelfPermission(Manifest.permission.BLUETOOTH_CONNECT) != PackageManager.PERMISSION_GRANTED
        ) return false
        val ask = Intent(BluetoothAdapter.ACTION_REQUEST_ENABLE).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
        DeviceActions.startOrNotify(app, ask, app.getString(R.string.radio_bt_prompt_title), app.getString(R.string.radio_body))
        return true
    }

    /** The settings screen where you switch [what]; null if the phone has none. */
    private fun panel(app: Context, what: String): Intent? {
        val q = Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q
        val candidates = when (what) {
            "wifi" -> listOfNotNull(if (q) Intent(Settings.Panel.ACTION_WIFI) else null, Intent(Settings.ACTION_WIFI_SETTINGS))
            "bluetooth" -> listOf(Intent(Settings.ACTION_BLUETOOTH_SETTINGS))
            "airplane" -> listOf(Intent(Settings.ACTION_AIRPLANE_MODE_SETTINGS))
            "data" -> listOfNotNull(
                if (q) Intent(Settings.Panel.ACTION_INTERNET_CONNECTIVITY) else null,
                Intent(Settings.ACTION_DATA_ROAMING_SETTINGS),
            )
            // No public screen for it: the AOSP one, then the network page.
            else -> listOf(
                Intent().setComponent(ComponentName("com.android.settings", "com.android.settings.TetherSettings")),
                Intent(Settings.ACTION_WIRELESS_SETTINGS),
            )
        } + Intent(Settings.ACTION_WIRELESS_SETTINGS)
        return candidates.firstOrNull { app.packageManager.resolveActivity(it, 0) != null }
            ?.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
    }

    /** WifiManager.WIFI_AP_STATE_CHANGED_ACTION (hidden). */
    private const val AP_STATE_CHANGED = "android.net.wifi.WIFI_AP_STATE_CHANGED"
}
