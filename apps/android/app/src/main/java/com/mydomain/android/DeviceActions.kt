package com.mydomain.android

import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.media.AudioAttributes
import android.media.AudioManager
import android.media.MediaPlayer
import android.media.RingtoneManager
import android.net.Uri
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.os.PowerManager
import android.provider.Settings
import android.util.Log
import androidx.core.app.NotificationCompat
import com.mydomain.android.core.Core

/**
 * The phone side of the core's quick device actions: ring (find my phone)
 * and open a link sent by another device. NetService calls these from the
 * `ring` / `open_url` core events, so they work with the app in the
 * background (the service runs; a push wakes it if it slept).
 */
object DeviceActions {
    private const val TAG = "mydomain_actions"
    const val CH_RING = "ring"
    const val CH_LINKS = "links"
    private const val RING_ID = 2001
    private const val RING_MS = 30_000L
    const val ACTION_STOP_RING = "com.mydomain.android.STOP_RING"

    private val main = Handler(Looper.getMainLooper())
    private val linkId = java.util.concurrent.atomic.AtomicInteger(3000)
    private var player: MediaPlayer? = null
    /** Alarm volume before ringing, restored when the ring stops. */
    private var savedVolume: Int? = null
    private val timeout = Runnable { stopRing() }

    fun createChannels(ctx: Context) {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.O) return
        val nm = ctx.getSystemService(NotificationManager::class.java) ?: return
        // No channel sound: the ring itself is a looping alarm-stream player
        // (a channel sound plays once, and would double it).
        val ring = NotificationChannel(CH_RING, ctx.getString(R.string.channel_ring), NotificationManager.IMPORTANCE_HIGH).apply {
            setSound(null, null)
            enableVibration(true)
            setBypassDnd(true) // honoured only if the user allows it
            lockscreenVisibility = android.app.Notification.VISIBILITY_PUBLIC
        }
        val links = NotificationChannel(CH_LINKS, ctx.getString(R.string.channel_links), NotificationManager.IMPORTANCE_HIGH)
        nm.createNotificationChannel(ring)
        nm.createNotificationChannel(links)
    }

    // --- ring ---------------------------------------------------------------

    /** Ring for up to 30 s at full alarm volume, with a Stop notification. */
    fun ring(ctx: Context, fromName: String) {
        val app = ctx.applicationContext
        main.post {
            startSound(app)
            postRingNotification(app, fromName)
            main.removeCallbacks(timeout)
            main.postDelayed(timeout, RING_MS)
        }
    }

    /** Stop the ring and remove its notification. Safe from any thread, and
     *  when nothing is ringing. */
    fun stopRing() {
        main.post {
            main.removeCallbacks(timeout)
            player?.let { p -> runCatching { p.stop() }; p.release() }
            player = null
            val am = runCatching { Core.app.getSystemService(AudioManager::class.java) }.getOrNull()
            savedVolume?.let { v -> runCatching { am?.setStreamVolume(AudioManager.STREAM_ALARM, v, 0) } }
            savedVolume = null
            runCatching { Core.app.getSystemService(NotificationManager::class.java)?.cancel(RING_ID) }
        }
    }

    private fun startSound(ctx: Context) {
        if (player != null) return // already ringing: just extend the timeout
        val am = ctx.getSystemService(AudioManager::class.java)
        if (am != null) {
            savedVolume = am.getStreamVolume(AudioManager.STREAM_ALARM)
            runCatching { am.setStreamVolume(AudioManager.STREAM_ALARM, am.getStreamMaxVolume(AudioManager.STREAM_ALARM), 0) }
        }
        val attrs = AudioAttributes.Builder()
            .setUsage(AudioAttributes.USAGE_ALARM)
            .setContentType(AudioAttributes.CONTENT_TYPE_SONIFICATION)
            .build()
        val tones = listOfNotNull(
            RingtoneManager.getActualDefaultRingtoneUri(ctx, RingtoneManager.TYPE_ALARM),
            RingtoneManager.getActualDefaultRingtoneUri(ctx, RingtoneManager.TYPE_RINGTONE),
            Settings.System.DEFAULT_ALARM_ALERT_URI,
            Settings.System.DEFAULT_RINGTONE_URI,
        )
        for (uri in tones) {
            val p = MediaPlayer()
            val ok = runCatching {
                p.setAudioAttributes(attrs)
                p.setDataSource(ctx, uri)
                p.isLooping = true
                p.setWakeMode(ctx, PowerManager.PARTIAL_WAKE_LOCK)
                p.prepare()
                p.start()
            }.onFailure { Log.w(TAG, "can't play $uri: $it") }.isSuccess
            if (ok) { player = p; return }
            p.release()
        }
        Log.w(TAG, "no tone could be played")
    }

    private fun postRingNotification(ctx: Context, fromName: String) {
        val flags = PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT
        val stop = PendingIntent.getBroadcast(
            ctx, 0, Intent(ctx, DeviceActionReceiver::class.java).setAction(ACTION_STOP_RING), flags,
        )
        // Tapping opens the app, and the app opening stops the ring.
        val open = PendingIntent.getActivity(
            ctx, 1,
            Intent(ctx, MainActivity::class.java).setAction(ACTION_STOP_RING)
                .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_SINGLE_TOP),
            flags,
        )
        val n = NotificationCompat.Builder(ctx, CH_RING)
            .setSmallIcon(android.R.drawable.ic_lock_idle_alarm)
            .setContentTitle(ctx.getString(R.string.ring_title, fromName))
            .setContentText(ctx.getString(R.string.ring_body))
            .setPriority(NotificationCompat.PRIORITY_MAX)
            .setCategory(NotificationCompat.CATEGORY_ALARM)
            .setVisibility(NotificationCompat.VISIBILITY_PUBLIC)
            .setOngoing(false)
            .setAutoCancel(true)
            .setTimeoutAfter(RING_MS)
            .setContentIntent(open)
            .setDeleteIntent(stop)
            .addAction(0, ctx.getString(R.string.ring_stop), stop)
            .build()
        runCatching { ctx.getSystemService(NotificationManager::class.java)?.notify(RING_ID, n) }
    }

    // --- open a link --------------------------------------------------------

    /** Only web links (the core already checks; this is the last gate before
     *  an ACTION_VIEW). */
    fun isWebLink(url: String): Boolean {
        val scheme = runCatching { Uri.parse(url).scheme?.lowercase() }.getOrNull()
        return scheme == "http" || scheme == "https"
    }

    /**
     * Open [url] from another device. On screen, open it straight away;
     * otherwise Android forbids starting an activity from the background, so
     * post a notification that opens it when tapped.
     */
    fun openUrl(ctx: Context, fromName: String, url: String) {
        if (!isWebLink(url)) return
        val app = ctx.applicationContext
        val view = Intent(Intent.ACTION_VIEW, Uri.parse(url)).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
        if (Core.visible) {
            val opened = runCatching { app.startActivity(view) }
                .onFailure { Log.w(TAG, "can't open link: $it") }.isSuccess
            if (opened) return
        }
        val id = linkId.incrementAndGet()
        val pi = PendingIntent.getActivity(app, id, view, PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
        val n = NotificationCompat.Builder(app, CH_LINKS)
            .setSmallIcon(android.R.drawable.ic_menu_view)
            .setContentTitle(app.getString(R.string.link_title, fromName))
            .setContentText(url)
            .setStyle(NotificationCompat.BigTextStyle().bigText(url))
            .setPriority(NotificationCompat.PRIORITY_HIGH)
            .setAutoCancel(true)
            .setContentIntent(pi)
            .build()
        runCatching { app.getSystemService(NotificationManager::class.java)?.notify(id, n) }
    }
}

/** The ring notification's Stop action and its dismissal. */
class DeviceActionReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        if (intent.action == DeviceActions.ACTION_STOP_RING) {
            Core.init(context)
            DeviceActions.stopRing()
        }
    }
}
