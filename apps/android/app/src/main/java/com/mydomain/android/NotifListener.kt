package com.mydomain.android

import android.app.Notification
import android.service.notification.NotificationListenerService
import android.service.notification.StatusBarNotification
import com.mydomain.android.core.Core
import com.mydomain.android.core.Tone
import com.mydomain.android.core.findCode

/**
 * Captures other apps' notifications and hands them to the Rust core, which
 * forwards each to the peers subscribed to that package (app-notification
 * pub/sub). The user must enable this under Settings → Notification access.
 *
 * With "Send verification codes to my computers" on, a one-time code found
 * in any notification (SMS, banks, Google, ...) also goes to the connected
 * devices, which copy it to their clipboard. The code is never logged.
 */
class NotifListener : NotificationListenerService() {

    override fun onNotificationPosted(sbn: StatusBarNotification?) {
        val n = sbn ?: return
        // Ignore our own + ongoing/group-summary noise.
        if (n.packageName == packageName) return
        val flags = n.notification?.flags ?: 0
        if (flags and Notification.FLAG_ONGOING_EVENT != 0) return
        if (flags and Notification.FLAG_GROUP_SUMMARY != 0) return

        val extras = n.notification?.extras ?: return
        val title = extras.getCharSequence(Notification.EXTRA_TITLE)?.toString().orEmpty()
        val text = extras.getCharSequence(Notification.EXTRA_TEXT)?.toString().orEmpty()
        if (title.isBlank() && text.isBlank()) return

        val label = appLabel(n.packageName)
        runCatching { Core.engine()?.shareAppNotification(n.packageName, label, title, text) }

        if (getSharedPreferences("md", MODE_PRIVATE).getBoolean(PREF_SHARE_CODES, false)) {
            val big = extras.getCharSequence(Notification.EXTRA_BIG_TEXT)?.toString().orEmpty()
            shareCode(listOf(title, text, big).joinToString("\n"), label)
        }
    }

    /** Send the verification code in [text], if any, unless the same code
     *  went out in the last 2 minutes (an SMS often shows up from two apps,
     *  and notifications get re-posted). */
    private fun shareCode(text: String, label: String) {
        val code = findCode(text) ?: return
        val now = System.currentTimeMillis()
        synchronized(recent) {
            recent.entries.removeAll { now - it.value > DEDUPE_MS }
            if (recent.containsKey(code)) return
        }
        val engine = Core.engine() ?: return
        if (runCatching { engine.shareCode(code, label) }.isFailure) return
        synchronized(recent) { recent[code] = now }
        Core.labLog("shared a verification code from $label")
        Core.addActivity(getString(R.string.act_code_sent), Tone.Ok)
    }

    companion object {
        /** SharedPreferences ("md") switch, default off. */
        const val PREF_SHARE_CODES = "share_codes"
        private const val DEDUPE_MS = 2 * 60 * 1000L

        /** Codes sent recently -> when (memory only). */
        private val recent = HashMap<String, Long>()
    }

    private fun appLabel(pkg: String): String = runCatching {
        val pm = packageManager
        pm.getApplicationLabel(pm.getApplicationInfo(pkg, 0)).toString()
    }.getOrDefault(pkg)
}
