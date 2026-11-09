package com.mydomain.android

import com.mydomain.android.core.NotificationIds
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import androidx.core.app.NotificationCompat
import com.mydomain.android.core.Core
import com.mydomain.android.core.RemoteDownloads
import org.json.JSONObject

/**
 * "Download on <device>": another device asks this phone to download a
 * link and the phone asks you first ([com.mydomain.android.core.RemoteDownloadPlan],
 * the core's "ask" setting). NetService calls [onEvent] with each
 * `remote_download` event, so the question shows with the app in the
 * background: a notification with Allow / Don't allow (answered by
 * [FileActionReceiver]); tapping it opens the app, which asks the same
 * (ui/RemoteDownloadUi.kt). Once answered, or ended any other way, the
 * notification goes. A downloaded file is never opened by itself.
 */
object DownloadAsk {
    const val ACTION_ALLOW = "com.mydomain.android.DL_ALLOW"
    const val ACTION_DENY = "com.mydomain.android.DL_DENY"

    private fun notifId(id: String) = NotificationIds.of(NotificationIds.Family.DOWNLOAD_ASK, id)

    fun onEvent(ctx: Context, o: JSONObject) {
        val item = o.optJSONObject("item")?.let(RemoteDownloads::parse) ?: return
        if (!item.asking) {
            dismiss(ctx, item.id)
            return
        }
        val device = Core.peerName(item.node)
        fun action(action: String, req: Int) = PendingIntent.getBroadcast(
            ctx, notifId(item.id) + req,
            Intent(ctx, FileActionReceiver::class.java).setAction(action).putExtra(FileActions.EXTRA_ID, item.id),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
        val open = PendingIntent.getActivity(
            ctx, notifId(item.id),
            Intent(ctx, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
        val n = NotificationCompat.Builder(ctx, FileActions.CH_FILES)
            .setSmallIcon(android.R.drawable.stat_sys_download)
            .setContentTitle(ctx.getString(R.string.dl_ask_title, device))
            .setContentText(ctx.getString(R.string.dl_ask_notif_body, item.name, item.host))
            .setAutoCancel(true)
            .setContentIntent(open)
            .addAction(0, ctx.getString(R.string.dl_ask_once), action(ACTION_ALLOW, 1))
            .addAction(0, ctx.getString(R.string.dl_ask_deny), action(ACTION_DENY, 2))
            .build()
        runCatching { ctx.getSystemService(NotificationManager::class.java)?.notify(notifId(item.id), n) }
    }

    fun dismiss(ctx: Context, id: String) {
        runCatching { ctx.getSystemService(NotificationManager::class.java)?.cancel(notifId(id)) }
    }

    /** From the notification's buttons. */
    fun answer(ctx: Context, id: String, allow: Boolean) {
        dismiss(ctx, id)
        Core.init(ctx)
        Thread { runCatching { Core.engine()?.remoteDownloadAnswer(id, allow, false) } }.start()
    }
}
