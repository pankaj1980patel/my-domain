package com.mydomain.android

import com.mydomain.android.core.AppScope
import com.mydomain.android.core.NotificationIds
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.os.Build
import androidx.core.app.NotificationCompat
import com.mydomain.android.core.Core
import org.json.JSONObject

/**
 * A computer's own notifications, shown on this phone (the reverse of
 * [NotifListener]'s mirror). The computer decides what it shares (its own
 * switch and per-app list); this phone asks for them, per computer, with
 * the core's `subscribe_computer_notifications` (a `subscribe_apps` for
 * the computer's one name, `*`).
 *
 * Only a computer whose status says `notif_share` is asked (an older one
 * would get nothing it understands), on by default for each such computer;
 * its page has the switch. The subscription lives in the computer's memory,
 * so it's asked again whenever a link to it opens or its status arrives.
 *
 * They post on one channel, "Computer notifications", grouped per computer.
 * Swiped away here, the computer closes it too (`notification_action`,
 * index -1); closed there, it goes away here (`notification_removed`).
 */
object ComputerNotifs {
    const val CHANNEL = "computer_notifs"
    private const val PREF_OFF = "computer_notifs_off"
    private const val PREF_CAPABLE = "computer_notifs_capable"
    const val ACTION_DISMISSED = "com.mydomain.android.COMPUTER_NOTIF_DISMISSED"
    const val EXTRA_FROM = "from"
    const val EXTRA_KEY = "key"
    private const val SUBSCRIBE_ALL = "[\"*\"]"

    /** Keys shown now, per computer (memory only), for the group summary. */
    private val shown = HashMap<String, MutableSet<String>>()
    /** Computers asked since their link last opened (memory only). */
    private val asked = HashSet<String>()

    private fun prefs(ctx: Context) = ctx.getSharedPreferences("md", Context.MODE_PRIVATE)

    fun createChannel(ctx: Context) {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.O) return
        val ch = NotificationChannel(CHANNEL, ctx.getString(R.string.channel_computer_notifs), NotificationManager.IMPORTANCE_DEFAULT)
        runCatching { ctx.getSystemService(NotificationManager::class.java)?.createNotificationChannel(ch) }
    }

    /** This computer said it can share its notifications (`notif_share`). */
    fun capable(ctx: Context, node: String): Boolean =
        prefs(ctx).getStringSet(PREF_CAPABLE, emptySet())!!.contains(node)

    /** Its notifications show here (on unless turned off on its page). */
    fun isOn(ctx: Context, node: String): Boolean =
        !prefs(ctx).getStringSet(PREF_OFF, emptySet())!!.contains(node)

    private fun editSet(ctx: Context, pref: String, node: String, add: Boolean) {
        val p = prefs(ctx)
        val now = p.getStringSet(pref, emptySet())!!.toMutableSet()
        if ((if (add) now.add(node) else now.remove(node))) p.edit().putStringSet(pref, now).apply()
    }

    /** The switch on the computer's page. Safe from the main thread. */
    fun setOn(ctx: Context, node: String, on: Boolean) {
        editSet(ctx, PREF_OFF, node, !on)
        if (!on) cancelAll(ctx, node)
        subscribe(node, on)
    }

    private fun subscribe(node: String, on: Boolean) {
        synchronized(asked) { if (on) asked.add(node) else asked.remove(node) }
        AppScope.bg("computer notifications subscribe") {
            runCatching { Core.engine()?.subscribeApps(node, if (on) SUBSCRIBE_ALL else "[]") }
                .onFailure { Core.labLog("computer notifications from $node: subscribe failed: ${it.javaClass.simpleName}") }
        }
    }

    /** A `device_status` from [node]: remember whether it can share, and
     *  ask for its notifications if that's new since its link opened. */
    fun onStatus(ctx: Context, node: String, status: JSONObject) {
        if (node.isEmpty()) return
        val can = status.has("notif_share")
        editSet(ctx, PREF_CAPABLE, node, can)
        if (!can || !isOn(ctx, node)) return
        if (synchronized(asked) { node in asked }) return
        subscribe(node, true)
    }

    /** A link to [node] opened: it may have restarted (and forgotten). */
    fun onConnected(ctx: Context, node: String) {
        if (node.isEmpty()) return
        synchronized(asked) { asked.remove(node) }
        if (capable(ctx, node) && isOn(ctx, node)) subscribe(node, true)
    }

    /** A `notification` event that is one of a computer's own. */
    fun handles(ctx: Context, o: JSONObject): Boolean {
        val key = o.optString("key")
        return (key.startsWith("fd:") || key.startsWith("gtk:")) && capable(ctx, o.optString("from"))
    }

    private fun idOf(node: String, key: String) = NotificationIds.of(NotificationIds.Family.COMPUTER_NOTIF, "$node|$key")
    private fun summaryId(node: String) = NotificationIds.of(NotificationIds.Family.COMPUTER_SUMMARY, node)
    private fun group(node: String) = "computer:$node"

    /** Show it (requires POST_NOTIFICATIONS on API 33+; a no-op without). */
    fun post(ctx: Context, o: JSONObject) {
        val from = o.optString("from")
        val key = o.optString("key")
        // Turned off here while one was on its way.
        if (!isOn(ctx, from)) return
        val app = o.optString("app")
        val title = o.optString("title")
        val body = o.optString("body")
        if (title.isBlank() && body.isBlank()) return
        val name = Core.peerName(from)
        val sub = if (app.isNotBlank()) ctx.getString(R.string.computer_notif_from_app, app, name)
        else ctx.getString(R.string.computer_notif_from, name)
        val id = idOf(from, key)
        val flags = PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT
        val dismissed = PendingIntent.getBroadcast(
            ctx, id,
            Intent(ctx, ComputerNotifReceiver::class.java).setAction(ACTION_DISMISSED)
                .putExtra(EXTRA_FROM, from).putExtra(EXTRA_KEY, key),
            flags,
        )
        val open = PendingIntent.getActivity(
            ctx, 0,
            Intent(ctx, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_SINGLE_TOP),
            flags,
        )
        val n = NotificationCompat.Builder(ctx, CHANNEL)
            .setSmallIcon(android.R.drawable.ic_dialog_info)
            .setContentTitle(title.ifBlank { app.ifBlank { name } })
            .setContentText(body)
            .setStyle(NotificationCompat.BigTextStyle().bigText(body))
            .setSubText(sub)
            .setGroup(group(from))
            .setOnlyAlertOnce(true)
            .setAutoCancel(true)
            .setContentIntent(open)
            .setDeleteIntent(dismissed)
            .build()
        val summary = NotificationCompat.Builder(ctx, CHANNEL)
            .setSmallIcon(android.R.drawable.ic_dialog_info)
            .setContentTitle(ctx.getString(R.string.computer_notif_from, name))
            .setSubText(name)
            .setGroup(group(from))
            .setGroupSummary(true)
            .setGroupAlertBehavior(NotificationCompat.GROUP_ALERT_CHILDREN)
            .setAutoCancel(true)
            .setContentIntent(open)
            .build()
        val nm = ctx.getSystemService(NotificationManager::class.java) ?: return
        runCatching {
            nm.notify(id, n)
            nm.notify(summaryId(from), summary)
            synchronized(shown) { shown.getOrPut(from) { HashSet() }.add(key) }
        }
    }

    /** Closed on the computer: gone here too. */
    fun remove(ctx: Context, from: String, key: String) {
        val nm = ctx.getSystemService(NotificationManager::class.java) ?: return
        runCatching { nm.cancel(idOf(from, key)) }
        forget(ctx, from, key)
    }

    /** Swiped away here: the computer closes it too (if it's still there). */
    fun dismissed(ctx: Context, from: String, key: String) {
        forget(ctx, from, key)
        AppScope.bg("computer notification dismiss") {
            runCatching { Core.engine()?.notificationAction(from, key, -1L, null) }
                .onFailure { Core.labLog("computer notification dismiss not sent: ${it.javaClass.simpleName}") }
        }
    }

    /** Drop [key]; the group's summary goes with its last notification. */
    private fun forget(ctx: Context, from: String, key: String) {
        val empty = synchronized(shown) {
            val keys = shown[from] ?: return
            keys.remove(key)
            if (keys.isEmpty()) shown.remove(from)
            keys.isEmpty()
        }
        if (empty) runCatching { ctx.getSystemService(NotificationManager::class.java)?.cancel(summaryId(from)) }
    }

    private fun cancelAll(ctx: Context, from: String) {
        val keys = synchronized(shown) { shown.remove(from) } ?: return
        val nm = ctx.getSystemService(NotificationManager::class.java) ?: return
        runCatching {
            keys.forEach { nm.cancel(idOf(from, it)) }
            nm.cancel(summaryId(from))
        }
    }
}

/** A computer's notification swiped away here ([ComputerNotifs.dismissed]). */
class ComputerNotifReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        if (intent.action != ComputerNotifs.ACTION_DISMISSED) return
        val from = intent.getStringExtra(ComputerNotifs.EXTRA_FROM) ?: return
        val key = intent.getStringExtra(ComputerNotifs.EXTRA_KEY) ?: return
        Core.init(context)
        ComputerNotifs.dismissed(context.applicationContext, from, key)
    }
}
