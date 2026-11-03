package com.mydomain.android

import android.app.Notification
import android.app.PendingIntent
import android.app.RemoteInput
import android.content.Intent
import android.os.Build
import android.os.Bundle
import android.service.notification.NotificationListenerService
import android.service.notification.StatusBarNotification
import com.mydomain.android.core.Core
import com.mydomain.android.core.Tone
import com.mydomain.android.core.findCode
import org.json.JSONArray
import org.json.JSONObject

/**
 * Captures other apps' notifications and hands them to the Rust core, which
 * forwards each to the peers subscribed to that package (app-notification
 * pub/sub). The user must enable this under Settings → Notification access.
 *
 * With "Send verification codes to my computers" on, a one-time code found
 * in any notification (SMS, banks, Google, ...) also goes to the connected
 * devices, which copy it to their clipboard. The code is never logged.
 *
 * Forwarded notifications carry their key and buttons, so another device
 * can press one, reply, or dismiss it ([onAction]).
 */
class NotifListener : NotificationListenerService() {

    override fun onListenerConnected() {
        super.onListenerConnected()
        // The access media sessions need is here now.
        NowPlaying.start(this)
        live = this
    }

    /** Do Not Disturb changed (the broadcast says so too; the core sends
     *  only real changes). */
    override fun onInterruptionFilterChanged(interruptionFilter: Int) {
        DndWatch.onFilter(interruptionFilter)
    }

    override fun onListenerDisconnected() {
        if (live === this) live = null
        super.onListenerDisconnected()
    }

    override fun onDestroy() {
        if (live === this) live = null
        super.onDestroy()
    }

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
        runCatching {
            Core.engine()?.shareAppNotificationWith(n.packageName, label, title, text, n.key, actionsJson(n.notification))
        }

        Core.labLog("notification from ${n.packageName}")
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
        // Logged without the code itself.
        val engine = Core.engine() ?: return Core.labLog("code from $label not sent: engine not running")
        runCatching { engine.shareCode(code, label) }.onFailure {
            Core.labLog("code from $label not sent: ${it.javaClass.simpleName}")
            return
        }
        synchronized(recent) { recent[code] = now }
        Core.labLog("shared a verification code from $label")
        Core.addActivity(getString(R.string.act_code_sent), Tone.Ok)
    }

    /** The notification's buttons, in order: `[{"title":..,"reply":..}]`;
     *  `reply` when the button takes typed text. */
    private fun actionsJson(notification: Notification): String {
        val arr = JSONArray()
        for (a in notification.actions.orEmpty()) {
            val reply = a.remoteInputs?.any { it.allowFreeFormInput } == true
            arr.put(JSONObject().put("title", a.title?.toString().orEmpty()).put("reply", reply))
        }
        return arr.toString()
    }

    /** Another device pressed button [index] of notification [key] (with
     *  [text] for a reply), or dismissed it (index -1). Logged without the
     *  text. */
    private fun act(from: String, key: String, index: Int, text: String?) {
        val sbn = runCatching { activeNotifications }.getOrNull()?.firstOrNull { it.key == key }
            ?: return Core.labLog("notification action from $from: notification gone")
        val label = appLabel(sbn.packageName)
        val device = Core.peerName(from)
        if (index == -1) {
            runCatching { cancelNotification(key) }
                .onFailure { return Core.labLog("dismiss failed: ${it.javaClass.simpleName}") }
            Core.labLog("dismissed a $label notification for $from")
            Core.addActivity(getString(R.string.act_notif_dismissed, label, device), Tone.Info)
            return
        }
        val action = sbn.notification.actions?.getOrNull(index)
            ?: return Core.labLog("notification action from $from: no button $index on $label")
        val pi = action.actionIntent
            ?: return Core.labLog("notification action from $from: button $index on $label has no intent")
        val inputs = action.remoteInputs
        val replied = !inputs.isNullOrEmpty() && text != null
        runCatching {
            if (replied) {
                val results = Bundle()
                for (r in inputs!!) if (r.allowFreeFormInput) results.putCharSequence(r.resultKey, text)
                val fill = Intent().addFlags(Intent.FLAG_RECEIVER_FOREGROUND)
                RemoteInput.addResultsToIntent(inputs, fill, results)
                if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.P) {
                    RemoteInput.setResultsSource(fill, RemoteInput.SOURCE_FREE_FORM_INPUT)
                }
                pi.send(this, 0, fill)
            } else {
                pi.send()
            }
        }.onFailure {
            val why = if (it is PendingIntent.CanceledException) "canceled" else it.javaClass.simpleName
            return Core.labLog("notification action on $label failed: $why")
        }
        Core.labLog("pressed button $index on $label for $from")
        val msg = if (replied) getString(R.string.act_notif_replied, label, device)
        else getString(R.string.act_notif_pressed, action.title?.toString().orEmpty(), label, device)
        Core.addActivity(msg, Tone.Ok)
    }

    companion object {
        /** The running listener (set while connected to the system). */
        @Volatile
        private var live: NotifListener? = null

        /** A `notification_action` core event: press/reply/dismiss on the
         *  live listener; logged and dropped when it isn't running. */
        fun onAction(o: JSONObject) {
            val from = o.optString("from")
            val l = live ?: return Core.labLog("notification action from $from: notification access off")
            val key = o.optString("key")
            if (key.isEmpty()) return Core.labLog("notification action from $from: no key")
            val text = if (o.isNull("text")) null else o.optString("text")
            l.act(from, key, o.optInt("index", -2).coerceAtLeast(-2), text)
        }

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
