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
    /** Open the dialer with [number] filled in (another device's `dial`
     *  action). It never calls by itself: you press Call on the phone.
     *  At once if this app is on screen, else a notification to tap. */
    fun dial(ctx: Context, fromName: String, number: String): Boolean {
        val clean = number.filter { it.isDigit() || it in "+*#" }
        if (clean.isEmpty() || clean.length > 20) return false
        val app = ctx.applicationContext
        val intent = Intent(Intent.ACTION_DIAL, Uri.parse("tel:" + Uri.encode(clean))).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
        if (Core.visible && runCatching { app.startActivity(intent) }.isSuccess) return true
        val id = linkId.incrementAndGet()
        val pi = PendingIntent.getActivity(app, id, intent, PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
        val n = NotificationCompat.Builder(app, CH_LINKS)
            .setSmallIcon(android.R.drawable.ic_menu_call)
            .setContentTitle(app.getString(R.string.dial_title, fromName, clean))
            .setContentText(app.getString(R.string.dial_body))
            .setPriority(NotificationCompat.PRIORITY_HIGH)
            .setAutoCancel(true)
            .setContentIntent(pi)
            .build()
        runCatching { app.getSystemService(NotificationManager::class.java)?.notify(id, n) }
        return true
    }

    /** Open app [pkg] (another device's `launch` action). Like a link: at
     *  once if this app is on screen, else a notification to tap (Android
     *  doesn't let a background app start another one). */
    fun launchApp(ctx: Context, fromName: String, pkg: String): Boolean {
        val app = ctx.applicationContext
        val pm = app.packageManager
        val launch = pm.getLaunchIntentForPackage(pkg)?.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK) ?: return false
        if (Core.visible && runCatching { app.startActivity(launch) }.isSuccess) return true
        val label = runCatching { pm.getApplicationLabel(pm.getApplicationInfo(pkg, 0)).toString() }.getOrDefault(pkg)
        val id = linkId.incrementAndGet()
        val pi = PendingIntent.getActivity(app, id, launch, PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
        val n = NotificationCompat.Builder(app, CH_LINKS)
            .setSmallIcon(android.R.drawable.ic_menu_view)
            .setContentTitle(app.getString(R.string.launch_title, fromName, label))
            .setContentText(app.getString(R.string.launch_body))
            .setPriority(NotificationCompat.PRIORITY_HIGH)
            .setAutoCancel(true)
            .setContentIntent(pi)
            .build()
        runCatching { app.getSystemService(NotificationManager::class.java)?.notify(id, n) }
        return true
    }

    /** Text from another device: "Copied from X" ([copied]) or "From X",
     *  with the text. Tapping opens it if it's a web link, else the app. */
    fun sharedText(ctx: Context, fromName: String, text: String, copied: Boolean) {
        val app = ctx.applicationContext
        val id = linkId.incrementAndGet()
        val link = text.trim().takeIf { isWebLink(it) && it.none(Char::isWhitespace) }
        val tap = if (link != null) {
            Intent(Intent.ACTION_VIEW, Uri.parse(link)).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
        } else {
            Intent(app, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_SINGLE_TOP)
        }
        val pi = PendingIntent.getActivity(app, id, tap, PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
        val short = text.take(1000)
        val n = NotificationCompat.Builder(app, CH_LINKS)
            .setSmallIcon(android.R.drawable.ic_menu_edit)
            .setContentTitle(app.getString(if (copied) R.string.text_copied_title else R.string.text_from_title, fromName))
            .setContentText(short)
            .setStyle(NotificationCompat.BigTextStyle().bigText(short))
            .setPriority(if (copied) NotificationCompat.PRIORITY_DEFAULT else NotificationCompat.PRIORITY_HIGH)
            .setAutoCancel(true)
            .setContentIntent(pi)
            .build()
        runCatching { app.getSystemService(NotificationManager::class.java)?.notify(id, n) }
    }

    /** Another device's battery is low ([alert] "low") or full ("full"). */
    fun batteryAlert(ctx: Context, fromName: String, alert: String, level: Int) {
        val app = ctx.applicationContext
        val low = alert == "low"
        val n = NotificationCompat.Builder(app, CH_LINKS)
            .setSmallIcon(android.R.drawable.ic_lock_idle_low_battery)
            .setContentTitle(app.getString(if (low) R.string.battery_low_title else R.string.battery_full_title, fromName))
            .setContentText(if (low) app.getString(R.string.battery_low_body, level) else app.getString(R.string.battery_full_body))
            .setPriority(NotificationCompat.PRIORITY_DEFAULT)
            .setAutoCancel(true)
            .build()
        // One notification per device and kind: a new one replaces the old.
        val id = 0x5000_0000 or ((fromName + alert).hashCode() and 0x0FFF_FFFF)
        runCatching { app.getSystemService(NotificationManager::class.java)?.notify(id, n) }
    }

    // --- keep awake ---------------------------------------------------------

    private var awakeLock: PowerManager.WakeLock? = null
    /** When the screen may go off again (ms since epoch), 0 = not held. */
    @Volatile var awakeUntil = 0L
        private set

    /** Keep the screen on for [minutes] (0 = stop), e.g. while a computer
     *  shows the gallery or a backup runs. A full wake lock with a timeout:
     *  it lets go by itself. */
    @Suppress("DEPRECATION", "WakelockTimeout")
    fun keepAwake(ctx: Context, minutes: Int): Boolean {
        synchronized(this) {
            awakeLock?.let { if (it.isHeld) runCatching { it.release() } }
            awakeLock = null
            awakeUntil = 0L
            if (minutes <= 0) return true
            val pm = ctx.applicationContext.getSystemService(PowerManager::class.java) ?: return false
            val ms = minutes.coerceAtMost(24 * 60) * 60_000L
            val lock = pm.newWakeLock(
                PowerManager.SCREEN_BRIGHT_WAKE_LOCK or PowerManager.ACQUIRE_CAUSES_WAKEUP,
                "mydomain:keep_awake",
            )
            lock.setReferenceCounted(false)
            lock.acquire(ms)
            awakeLock = lock
            awakeUntil = System.currentTimeMillis() + ms
            return true
        }
    }

    // --- play on this phone ---------------------------------------------------

    private const val YOUTUBE = "com.google.android.youtube"
    private const val SPOTIFY = "com.spotify.music"

    private fun installed(ctx: Context, pkg: String): Boolean =
        runCatching { ctx.packageManager.getPackageInfo(pkg, 0) }.isSuccess

    /** The `search` apps this phone has (the web always works). */
    fun searchApps(ctx: Context): org.json.JSONArray = org.json.JSONArray().apply {
        if (installed(ctx, YOUTUBE)) put("youtube")
        if (installed(ctx, SPOTIFY)) put("spotify")
        put("web")
    }

    /** Another device's `search {app, query}`: the app's own search when
     *  it's installed, else the same search in the browser. */
    fun search(ctx: Context, fromName: String, appName: String, query: String): Boolean {
        val q = query.trim().take(500)
        if (q.isEmpty()) return false
        val app = ctx.applicationContext
        val enc = Uri.encode(q)
        val intent = when (appName) {
            "youtube" -> if (installed(app, YOUTUBE)) {
                Intent(Intent.ACTION_SEARCH).setPackage(YOUTUBE).putExtra("query", q)
            } else Intent(Intent.ACTION_VIEW, Uri.parse("https://www.youtube.com/results?search_query=$enc"))
            "spotify" -> if (installed(app, SPOTIFY)) {
                Intent(Intent.ACTION_VIEW, Uri.parse("spotify:search:$enc")).setPackage(SPOTIFY)
            } else Intent(Intent.ACTION_VIEW, Uri.parse("https://open.spotify.com/search/$enc"))
            "web" -> Intent(Intent.ACTION_WEB_SEARCH).putExtra(android.app.SearchManager.QUERY, q)
            else -> return false
        }.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
        // A web search nobody handles: a search page in the browser.
        val target = if (intent.resolveActivity(app.packageManager) != null) intent
        else Intent(Intent.ACTION_VIEW, Uri.parse("https://duckduckgo.com/?q=$enc")).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
        startOrNotify(app, target, app.getString(R.string.search_title, fromName, q), app.getString(R.string.launch_body))
        return true
    }

    /** At once if this app is on screen; else a notification to tap
     *  (Android doesn't let a background app start an activity). */
    fun startOrNotify(app: Context, intent: Intent, title: String, text: String) {
        if (Core.visible && runCatching { app.startActivity(intent) }.isSuccess) return
        val id = linkId.incrementAndGet()
        val pi = PendingIntent.getActivity(app, id, intent, PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
        val n = NotificationCompat.Builder(app, CH_LINKS)
            .setSmallIcon(android.R.drawable.ic_media_play)
            .setContentTitle(title)
            .setContentText(text)
            .setPriority(NotificationCompat.PRIORITY_HIGH)
            .setAutoCancel(true)
            .setContentIntent(pi)
            .build()
        runCatching { app.getSystemService(NotificationManager::class.java)?.notify(id, n) }
    }

    /**
     * Open [url] from another device, in app [pkg] when it's given, installed
     * and takes the link (else the browser). On screen, open it straight
     * away; otherwise Android forbids starting an activity from the
     * background, so post a notification that opens it when tapped.
     */
    fun openUrl(ctx: Context, fromName: String, url: String, pkg: String? = null) {
        if (!isWebLink(url)) return
        val app = ctx.applicationContext
        val plain = Intent(Intent.ACTION_VIEW, Uri.parse(url)).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
        val inApp = pkg?.takeIf { installed(app, it) }?.let { Intent(plain).setPackage(it) }
        val view = inApp?.takeIf { it.resolveActivity(app.packageManager) != null } ?: plain
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
