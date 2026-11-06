package com.mydomain.android

import android.app.KeyguardManager
import android.app.Notification
import android.app.PendingIntent
import android.app.RemoteInput
import android.content.Context
import android.content.Intent
import android.graphics.Bitmap
import android.graphics.Canvas
import android.graphics.Color
import android.graphics.drawable.BitmapDrawable
import android.graphics.drawable.Drawable
import android.graphics.drawable.Icon
import android.os.Build
import android.os.Bundle
import android.os.PowerManager
import android.service.notification.NotificationListenerService
import android.service.notification.StatusBarNotification
import android.util.Base64
import androidx.core.app.NotificationCompat
import com.mydomain.android.core.Core
import com.mydomain.android.core.NotifDedupe
import com.mydomain.android.core.NotifFacts
import com.mydomain.android.core.NotifFilter
import com.mydomain.android.core.NotifSkip
import com.mydomain.android.core.Tone
import com.mydomain.android.core.MIN_CONFIDENCE
import com.mydomain.android.core.codeTargets
import com.mydomain.android.core.detectCode
import com.mydomain.android.core.notifFingerprint
import java.io.ByteArrayOutputStream
import org.json.JSONArray
import org.json.JSONObject

/**
 * Captures other apps' notifications and hands them to the Rust core, which
 * forwards each to the peers subscribed to that package (app-notification
 * pub/sub). The user must enable this under Settings → Notification access.
 *
 * With "Send verification codes to my computers" on (off by default), a
 * one-time code found in a notification (messaging, bank and sign-in apps;
 * never read from SMS itself) goes to the computers chosen: only the code,
 * the app, who it's from and when it expires, never the notification's
 * text. The code is never logged, nor kept in the activity list.
 *
 * Forwarded notifications carry their key and buttons, so another device
 * can press one, reply, or dismiss it ([onAction]); when one goes away here
 * the subscribed devices hear so ([onNotificationRemoved]).
 *
 * What goes ([NotifFilter]): not ongoing ones, group summaries, foreground
 * services, media players, low-importance channels or the system's own
 * status; the same one posted again with nothing new isn't sent again
 * ([NotifDedupe]), an update goes as `silent`. Each carries its
 * conversation (the last few messages, who wrote them), the sender's
 * picture and any picture in it as small JPEGs, the app's icon (sent once,
 * on request), whether it looks sensitive (a code, a bank, a password
 * manager: a computer in private mode then gets no content) and whether
 * the phone is in use now (for computers that want them only while it
 * isn't). The core sends each computer what it understands.
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
        val notification = n.notification ?: return
        val extras = notification.extras ?: return
        val title = extras.getCharSequence(Notification.EXTRA_TITLE)?.toString().orEmpty()
        val text = extras.getCharSequence(Notification.EXTRA_TEXT)?.toString().orEmpty()
        val flags = notification.flags
        val facts = NotifFacts(
            pkg = n.packageName,
            ownPkg = packageName,
            ongoing = flags and Notification.FLAG_ONGOING_EVENT != 0,
            groupSummary = flags and Notification.FLAG_GROUP_SUMMARY != 0,
            foregroundService = flags and Notification.FLAG_FOREGROUND_SERVICE != 0,
            localOnly = flags and Notification.FLAG_LOCAL_ONLY != 0,
            importance = importanceOf(n.key),
            category = notification.category,
            template = extras.getString(Notification.EXTRA_TEMPLATE),
            title = title,
            text = text,
        )
        val skip = NotifFilter.skip(facts)
        // A code in a quiet one (a low-importance channel, say) still goes
        // to the clipboard as before; only these never count.
        if (skip in setOf(NotifSkip.OWN, NotifSkip.ONGOING, NotifSkip.SUMMARY, NotifSkip.EMPTY)) return

        val label = appLabel(n.packageName)
        if (skip == NotifSkip.NONE) runCatching {
            val engine = Core.engine() ?: return@runCatching
            val rich = richJson(n, label, title, text) ?: return@runCatching
            if (sentIcons.add(n.packageName)) appIconJpeg(n.packageName)?.let { engine.setAppIcon(n.packageName, it) }
            engine.shareAppNotificationRich(n.packageName, rich.toString())
            synchronized(shared) { shared[n.key] = n.packageName }
        }

        Core.labLog("notification from ${n.packageName}")
        if (getSharedPreferences("md", MODE_PRIVATE).getBoolean(PREF_SHARE_CODES, false)) {
            val big = extras.getCharSequence(Notification.EXTRA_BIG_TEXT)?.toString().orEmpty()
            shareCode(title, listOf(text, big).filter { it.isNotBlank() }.distinct().joinToString("\n"), label)
        }
    }

    /** A notification went away here (swiped, opened, cancelled by its
     *  app). Only one that was shared (it passed the filters above) is
     *  reported; the core tells just the devices subscribed to its app. */
    override fun onNotificationRemoved(sbn: StatusBarNotification?) {
        val n = sbn ?: return
        dedupe.forget(n.key)
        val pkg = synchronized(shared) { shared.remove(n.key) } ?: return
        runCatching { Core.engine()?.shareAppNotificationRemoved(pkg, n.key) }
    }

    /** Send the verification code in [text] (titled [title]), if one is
     *  there surely enough, to the computers chosen, unless the same code
     *  went out in the last 2 minutes (an SMS often shows up from two apps,
     *  and notifications get re-posted). */
    private fun shareCode(title: String, text: String, label: String) {
        val found = detectCode(text, title, label) ?: return
        if (found.confidence < MIN_CONFIDENCE) return
        val code = found.code
        val now = System.currentTimeMillis()
        synchronized(recent) {
            recent.entries.removeAll { now - it.value > DEDUPE_MS }
            if (recent.containsKey(code)) return
        }
        // Logged without the code itself.
        val engine = Core.engine() ?: return Core.labLog("code from $label not sent: engine not running")
        val prefs = getSharedPreferences("md", MODE_PRIVATE)
        val targets = codeTargets(codeComputers(engine.verificationCodeComputersJson()), chosenComputers(prefs))
        if (targets.isEmpty()) return Core.labLog("code from $label not sent: no computer takes codes")
        runCatching { engine.sendVerificationCode(code, label, found.issuer, targets) }.onFailure {
            Core.labLog("code from $label not sent: ${it.javaClass.simpleName}")
            return
        }
        synchronized(recent) { recent[code] = now }
        Core.labLog("sent a verification code from $label")
    }

    /** The notification's buttons, in order: `[{"title":..,"reply":..}]`;
     *  `reply` when the button takes typed text. */
    private fun actionsJson(notification: Notification): JSONArray {
        val arr = JSONArray()
        for (a in notification.actions.orEmpty()) {
            val reply = a.remoteInputs?.any { it.allowFreeFormInput } == true
            // The system's own suggestions keep their place (indexes are
            // the phone's) but have no title, so they're not shown.
            val contextual = Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q && a.isContextual
            val title = if (contextual) "" else a.title?.toString().orEmpty()
            arr.put(JSONObject().put("title", title).put("reply", reply && !contextual))
        }
        return arr
    }

    /** The channel's importance, from the ranking (unknown: unspecified). */
    private fun importanceOf(key: String): Int = runCatching {
        val r = Ranking()
        if (currentRanking?.getRanking(key, r) == true) r.importance else NotifFilter.IMPORTANCE_UNSPECIFIED
    }.getOrDefault(NotifFilter.IMPORTANCE_UNSPECIFIED)

    /** Unlocked with the screen on. */
    private fun inUse(): Boolean {
        val pm = getSystemService(Context.POWER_SERVICE) as? PowerManager
        val km = getSystemService(Context.KEYGUARD_SERVICE) as? KeyguardManager
        return pm?.isInteractive == true && km?.isKeyguardLocked != true
    }

    /** All the core sends (see the class docs), or null when it's the
     *  same one again with nothing new. */
    private fun richJson(n: StatusBarNotification, label: String, title: String, text: String): JSONObject? {
        val notification = n.notification
        val extras = notification.extras
        val style = runCatching { NotificationCompat.MessagingStyle.extractMessagingStyleFromNotification(notification) }
            .getOrNull()
        val msgs = style?.messages.orEmpty().takeLast(MAX_MESSAGES)
        val big = extras.getCharSequence(Notification.EXTRA_BIG_TEXT)?.toString()
        val body = if (!big.isNullOrBlank() && big.length > text.length) big else text
        val picture = pictureOf(extras)
        val lines = msgs.map { "${it.person?.name}: ${it.text}" }
        val seen = dedupe.seen(n.key, notifFingerprint(title, body, lines, picture != null))
        if (seen == NotifDedupe.Seen.REPEAT) return null

        val o = JSONObject()
            .put("app", label)
            .put("title", title)
            .put("body", body)
            .put("key", n.key)
            .put("actions", actionsJson(notification))
            .put("ts", n.postTime)
            .put("in_use", inUse())
        val alertOnce = notification.flags and Notification.FLAG_ONLY_ALERT_ONCE != 0
        if (seen == NotifDedupe.Seen.UPDATE && alertOnce) o.put("silent", true)
        val said = (listOf(body) + msgs.map { it.text?.toString().orEmpty() }).joinToString(" ")
        if (NotifFilter.isSensitive(n.packageName, label, title, said, notification.visibility)) o.put("sensitive", true)
        // The conversation: its id (the shortcut, else its name), its name,
        // who wrote last, and the last few messages.
        val convTitle = style?.conversationTitle?.toString()
        val shortcut = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) notification.shortcutId else null
        val conv = shortcut ?: convTitle ?: if (style != null) title else null
        conv?.let { o.put("conv", it) }
        convTitle?.let { o.put("conv_title", it) }
        if (style?.isGroupConversation == true) o.put("group_chat", true)
        val last = msgs.lastOrNull()
        val sender = last?.person?.name?.toString() ?: if (style == null) title else null
        sender?.let { o.put("sender", it) }
        if (msgs.isNotEmpty()) {
            val arr = JSONArray()
            for (m in msgs) {
                val who = m.person?.name?.toString() ?: style?.user?.name?.toString().orEmpty()
                arr.put(JSONObject().put("sender", who).put("text", m.text?.toString().orEmpty()).put("ts", m.timestamp))
            }
            o.put("messages", arr)
        }
        // The sender's picture: theirs in the conversation, else the large icon.
        val avatar = runCatching { last?.person?.icon?.loadDrawable(this) }.getOrNull()
            ?: runCatching { notification.getLargeIcon()?.loadDrawable(this) }.getOrNull()
        avatar?.let { runCatching { jpegOfDrawable(it, AVATAR_PX, AVATAR_BYTES) }.getOrNull() }
            ?.let { o.put("avatar", it) }
        picture?.let { runCatching { jpegOf(it, PICTURE_PX, PICTURE_BYTES) }.getOrNull() }?.let { o.put("picture", it) }
        return o
    }

    /** A picture in the notification (big picture style). */
    @Suppress("DEPRECATION")
    private fun pictureOf(extras: Bundle): Bitmap? = runCatching {
        (extras.get(Notification.EXTRA_PICTURE) as? Bitmap)
            ?: if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
                (extras.getParcelable(Notification.EXTRA_PICTURE_ICON) as? Icon)
                    ?.loadDrawable(this)?.let { bitmapOf(it, PICTURE_PX) }
            } else {
                null
            }
    }.getOrNull()

    /** App [pkg]'s icon as a small JPEG, base64. */
    private fun appIconJpeg(pkg: String): String? = runCatching {
        jpegOfDrawable(packageManager.getApplicationIcon(pkg), ICON_PX, ICON_BYTES)
    }.getOrNull()

    /** [d] as a JPEG ([jpegOf]); a bitmap drawn for it here is recycled. */
    private fun jpegOfDrawable(d: Drawable, px: Int, maxBytes: Int): String? {
        val b = bitmapOf(d, px)
        return try {
            jpegOf(b, px, maxBytes)
        } finally {
            if (!(d is BitmapDrawable && d.bitmap === b)) b.recycle()
        }
    }

    private fun bitmapOf(d: Drawable, px: Int): Bitmap {
        if (d is BitmapDrawable && d.bitmap != null) return d.bitmap
        val w = d.intrinsicWidth.takeIf { it > 0 } ?: px
        val h = d.intrinsicHeight.takeIf { it > 0 } ?: px
        val b = Bitmap.createBitmap(w, h, Bitmap.Config.ARGB_8888)
        d.setBounds(0, 0, w, h)
        d.draw(Canvas(b))
        return b
    }

    /** [src] scaled to fit [px], on white (JPEG has no transparency), as a
     *  JPEG of at most [maxBytes], base64; null when it won't fit. */
    private fun jpegOf(src: Bitmap, px: Int, maxBytes: Int): String? {
        val scale = minOf(1f, px.toFloat() / maxOf(src.width, src.height, 1))
        val w = maxOf(1, (src.width * scale).toInt())
        val h = maxOf(1, (src.height * scale).toInt())
        val out = Bitmap.createBitmap(w, h, Bitmap.Config.ARGB_8888)
        try {
            val scaled = Bitmap.createScaledBitmap(src, w, h, true)
            Canvas(out).apply {
                drawColor(Color.WHITE)
                drawBitmap(scaled, 0f, 0f, null)
            }
            if (scaled !== src) scaled.recycle()
            for (q in intArrayOf(80, 65, 50, 35)) {
                val bytes = ByteArrayOutputStream().also { out.compress(Bitmap.CompressFormat.JPEG, q, it) }.toByteArray()
                if (bytes.size <= maxBytes) return Base64.encodeToString(bytes, Base64.NO_WRAP)
            }
            return null
        } finally {
            out.recycle()
        }
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

        /** What each notification last said (memory only). */
        private val dedupe = NotifDedupe()

        /** Packages whose icon the core has (memory only, like its own). */
        private val sentIcons: MutableSet<String> = java.util.Collections.synchronizedSet(HashSet())

        private const val MAX_MESSAGES = 6
        private const val AVATAR_PX = 96
        private const val AVATAR_BYTES = 12 * 1024
        private const val PICTURE_PX = 360
        private const val PICTURE_BYTES = 44 * 1024
        private const val ICON_PX = 72
        private const val ICON_BYTES = 12 * 1024

        /** SharedPreferences ("md") switch, default off. */
        const val PREF_SHARE_CODES = "share_codes"
        /** The computers chosen for codes (a string set of node ids);
         *  unset: all your computers. */
        const val PREF_CODE_COMPUTERS = "code_computers"

        /** The chosen computers, or null for all of them. */
        fun chosenComputers(prefs: android.content.SharedPreferences): Set<String>? =
            if (prefs.contains(PREF_CODE_COMPUTERS)) prefs.getStringSet(PREF_CODE_COMPUTERS, emptySet()).orEmpty() else null

        /** `MdEngine.verificationCodeComputersJson` as (node id, takes codes). */
        fun codeComputers(json: String): List<Pair<String, Boolean>> = runCatching {
            val arr = JSONArray(json)
            (0 until arr.length()).map { arr.getJSONObject(it).let { o -> o.optString("node") to o.optBoolean("supported") } }
        }.getOrDefault(emptyList())
        private const val DEDUPE_MS = 2 * 60 * 1000L

        /** Codes sent recently -> when (memory only). */
        private val recent = HashMap<String, Long>()

        /** Keys of the notifications shared, -> their package (memory only;
         *  the oldest go first past [MAX_SHARED]), so a removal is reported
         *  only for one that went out. */
        private const val MAX_SHARED = 500
        private val shared = object : LinkedHashMap<String, String>() {
            override fun removeEldestEntry(eldest: MutableMap.MutableEntry<String, String>?): Boolean =
                size > MAX_SHARED
        }
    }

    private fun appLabel(pkg: String): String = runCatching {
        val pm = packageManager
        pm.getApplicationLabel(pm.getApplicationInfo(pkg, 0)).toString()
    }.getOrDefault(pkg)
}
