package com.mydomain.android

import android.app.Activity
import android.app.KeyguardManager
import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.content.pm.ServiceInfo
import android.graphics.Bitmap
import android.graphics.PixelFormat
import android.hardware.display.DisplayManager
import android.hardware.display.VirtualDisplay
import android.media.ImageReader
import android.media.projection.MediaProjection
import android.media.projection.MediaProjectionManager
import android.os.Build
import android.os.Bundle
import android.os.Handler
import android.os.HandlerThread
import android.os.IBinder
import android.os.Looper
import android.util.DisplayMetrics
import android.util.Log
import android.view.WindowManager
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.core.app.NotificationCompat
import com.mydomain.android.core.Core
import com.mydomain.android.core.ScreenTouch
import org.json.JSONArray
import java.io.ByteArrayOutputStream
import com.mydomain.android.core.ResourceBook
import com.mydomain.android.core.ResourceLedger

/**
 * Your computers see this phone's screen (mdcore `features::screen`, the
 * other way round from the touchpad's screen view). Two ways in:
 *
 *  - a computer asks to watch: the phone asks you (a prompt on screen, else
 *    a notification), then Android's own "start recording or casting?"
 *    prompt, every time;
 *  - you tap "Share this screen with …" on a computer's page here: Android's
 *    prompt, then the phone offers it to that computer (`screen_offer`),
 *    which starts watching.
 *
 * A computer may watch only if allowed for it (mdcore `features::perms`,
 * "screen"): always (the switch on its page), or once (sharing from its
 * page, or "Allow once" when it asks: [PeerPerms]). Android's prompt comes
 * every time all the same.
 *
 * Nothing is captured without Android's prompt answered yes. While it
 * shares, a notification says with whom, with Stop and "Allow control"
 * (off at each start: view only). It stops on Stop, when the computers stop
 * watching or their link goes (the watch runs out), and when the screen goes
 * off or locks: the lock screen is never sent. Snapshots are small JPEGs
 * about twice a second, sent only to the computers you shared with (never
 * kept on the phone).
 *
 * With "Allow control" on and the Accessibility helper on, taps and swipes
 * on the computer's picture are played here ([onInput], [PhoneHelper.touch]).
 */
object ScreenShare {
    private const val TAG = "mydomain_screen"
    private const val CH = "screen_share"
    const val NOTE_ASK = 7301
    const val NOTE_SHARING = 7302

    /** Computers this screen is shared with now (empty: not sharing). For
     *  the device screen; the service keeps the real list. */
    var sharingWith by mutableStateOf<Set<String>>(emptySet())
        private set
    /** They may tap and swipe here (the notification's "Allow control"). */
    var control by mutableStateOf(false)
        private set

    private val main = Handler(Looper.getMainLooper())

    internal fun published(viewers: Set<String>, allowControl: Boolean) {
        main.post {
            sharingWith = viewers
            control = allowControl
        }
        Controls.onChange()
    }

    /** A computer started ([on]) or stopped watching this screen. */
    fun onWatch(ctx: Context, from: String, on: Boolean) {
        if (!on) {
            ScreenShareService.current?.viewerLeft(from)
            return
        }
        val svc = ScreenShareService.current
        if (svc != null && svc.isViewer(from)) return
        Log.i(TAG, "asked to share the screen (app on screen: ${Core.visible}, sharing: ${svc != null})")
        val app = ctx.applicationContext
        channel(app)
        val ask = Intent(app, ScreenConsentActivity::class.java)
            .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
            .putExtra(ScreenConsentActivity.EXTRA_NODE, from)
        // Already sharing with another computer: ask by notification only
        // (never pop a screen over what you're showing).
        if (svc == null && Core.visible && runCatching { app.startActivity(ask) }.onFailure { Log.w(TAG, "can't open the prompt: $it") }.isSuccess) return
        val pi = PendingIntent.getActivity(app, NOTE_ASK, ask, PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
        val title = if (svc == null) R.string.screen_ask_title else R.string.screen_also_ask_title
        val n = NotificationCompat.Builder(app, CH)
            .setSmallIcon(android.R.drawable.ic_menu_view)
            .setContentTitle(app.getString(title, Core.peerName(from)))
            .setContentText(app.getString(if (svc == null) R.string.screen_ask_body else R.string.screen_also_ask_body))
            .setPriority(NotificationCompat.PRIORITY_HIGH)
            .setAutoCancel(true)
            .setContentIntent(pi)
            .build()
        runCatching { app.getSystemService(NotificationManager::class.java)?.notify(NOTE_ASK, n) }
            .onFailure { Log.w(TAG, "can't ask: $it") }
    }

    /** You chose to share this screen with computer [node] (from its page;
     *  [ctx] is the activity): Android's prompt, then the offer. */
    fun share(ctx: Context, node: String) {
        // Your choice: this computer may see it this time (or always, if
        // its page says so).
        PeerPerms.allowForSharing(node)
        val svc = ScreenShareService.current
        if (svc != null) {
            svc.addViewer(node, offer = true)
            return
        }
        ctx.startActivity(
            Intent(ctx, ScreenConsentActivity::class.java)
                .putExtra(ScreenConsentActivity.EXTRA_NODE, node)
                .putExtra(ScreenConsentActivity.EXTRA_OFFER, true),
        )
    }

    /** Stop sharing with [node] (the last one: stop sharing). */
    fun stopWith(node: String) {
        ScreenShareService.current?.viewerLeft(node, tell = true)
    }

    /** Allow or forbid taps and swipes from the computers it shares with. */
    fun allowControl(on: Boolean) {
        ScreenShareService.current?.setControl(on)
    }

    /** Input events from computer [from] (core `input`): taps and swipes on
     *  its picture of this screen. Played only while this screen is shared
     *  with it, with "Allow control" on and the helper on. */
    fun onInput(ctx: Context, from: String, events: JSONArray?) {
        val svc = ScreenShareService.current ?: return
        if (events == null || !svc.isViewer(from)) return
        // Secure mode: say so here (once a while), with a way to turn it off.
        val helper = PhoneHelper.instance
        if (helper == null) {
            SecureMode.blocked(ctx, Core.peerName(from))
            return
        }
        if (!svc.controlAllowed) return
        val real = realSize(ctx)
        val (fw, fh) = svc.frameSize
        for (i in 0 until events.length()) {
            val e = events.optJSONObject(i) ?: continue
            val kind = e.optString("kind")
            if (kind != "tap" && kind != "swipe") continue
            val from0 = ScreenTouch.toScreen(e.optInt("x"), e.optInt("y"), fw, fh, real.first, real.second) ?: continue
            val to = if (kind == "swipe") {
                ScreenTouch.toScreen(e.optInt("x2"), e.optInt("y2"), fw, fh, real.first, real.second) ?: continue
            } else from0
            val ms = if (kind == "swipe") e.optInt("ms", 300).toLong() else TAP_MS
            if (!helper.touch(from0.first, from0.second, to.first, to.second, ms)) Log.w(TAG, "$kind not played")
        }
    }

    private const val TAP_MS = 50L

    /** The whole screen in pixels, as it's turned now. */
    @Suppress("DEPRECATION")
    fun realSize(ctx: Context): Pair<Int, Int> {
        val m = DisplayMetrics()
        ctx.getSystemService(WindowManager::class.java)?.defaultDisplay?.getRealMetrics(m)
        if (m.widthPixels <= 0) return ctx.resources.displayMetrics.let { it.widthPixels to it.heightPixels }
        return m.widthPixels to m.heightPixels
    }

    fun channel(ctx: Context) {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.O) return
        ctx.getSystemService(NotificationManager::class.java)?.createNotificationChannel(
            NotificationChannel(CH, ctx.getString(R.string.channel_screen), NotificationManager.IMPORTANCE_HIGH),
        )
    }

    /** "Sharing your screen with …", Stop, and Allow control (with the
     *  helper on) or View only. */
    fun sharingNote(ctx: Context, viewers: Set<String>, allowControl: Boolean): Notification {
        val stop = PendingIntent.getService(
            ctx, 0, Intent(ctx, ScreenShareService::class.java).setAction(ScreenShareService.ACTION_STOP),
            PendingIntent.FLAG_IMMUTABLE,
        )
        val names = viewers.joinToString(", ") { Core.peerName(it) }
        val b = NotificationCompat.Builder(ctx, CH)
            .setSmallIcon(android.R.drawable.ic_menu_view)
            .setContentTitle(
                if (names.isEmpty()) ctx.getString(R.string.screen_sharing_title)
                else ctx.getString(R.string.screen_sharing_with, names),
            )
            .setContentText(ctx.getString(if (allowControl) R.string.screen_sharing_body_control else R.string.screen_sharing_body))
            .setOngoing(true)
            .setOnlyAlertOnce(true)
            .addAction(0, ctx.getString(R.string.screen_stop), stop)
        if (PhoneHelper.instance != null || allowControl) {
            val toggle = PendingIntent.getService(
                ctx, 1,
                Intent(ctx, ScreenShareService::class.java)
                    .setAction(ScreenShareService.ACTION_CONTROL)
                    .putExtra(ScreenShareService.EXTRA_ON, !allowControl),
                PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
            )
            b.addAction(0, ctx.getString(if (allowControl) R.string.screen_view_only else R.string.screen_allow_control), toggle)
        }
        return b.build()
    }
}

/** Shows Android's screen-recording prompt; a yes starts the sharing. */
class ScreenConsentActivity : Activity() {
    private var node = ""
    private var offer = false

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        getSystemService(NotificationManager::class.java)?.cancel(ScreenShare.NOTE_ASK)
        node = intent?.getStringExtra(EXTRA_NODE).orEmpty()
        offer = intent?.getBooleanExtra(EXTRA_OFFER, false) == true
        // Already sharing (Android said yes then): you tapped to let this
        // computer see it too.
        ScreenShareService.current?.let { svc ->
            if (node.isNotEmpty()) svc.addViewer(node, offer)
            return finish()
        }
        // One capture at a time: this phone's audio is playing on a
        // computer (a second MediaProjection would end it).
        if (PhoneAudioService.running) {
            android.widget.Toast.makeText(this, getString(R.string.screen_busy_audio), android.widget.Toast.LENGTH_LONG).show()
            return finish()
        }
        val mpm = getSystemService(MediaProjectionManager::class.java) ?: return finish()
        @Suppress("DEPRECATION")
        startActivityForResult(mpm.createScreenCaptureIntent(), ASK)
    }

    @Deprecated("Activity result API; this activity has no other result")
    override fun onActivityResult(requestCode: Int, resultCode: Int, data: Intent?) {
        @Suppress("DEPRECATION")
        super.onActivityResult(requestCode, resultCode, data)
        Log.i("mydomain_screen", "prompt answered: ${if (resultCode == RESULT_OK) "yes" else "no ($resultCode)"}")
        if (requestCode == ASK && resultCode == RESULT_OK && data != null && node.isNotEmpty()) {
            val start = Intent(this, ScreenShareService::class.java)
                .putExtra(ScreenShareService.EXTRA_CODE, resultCode)
                .putExtra(ScreenShareService.EXTRA_DATA, data)
                .putExtra(EXTRA_NODE, node)
                .putExtra(EXTRA_OFFER, offer)
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) startForegroundService(start) else startService(start)
        }
        finish()
    }

    companion object {
        /** The computer to share with (its node id). */
        const val EXTRA_NODE = "node"
        /** Offer it to that computer once it runs (you started it here). */
        const val EXTRA_OFFER = "offer"
        private const val ASK = 1
    }
}

/** Takes the snapshots and sends them to the computers you shared with. */
class ScreenShareService : Service() {
    private var projection: MediaProjection? = null
    private var display: VirtualDisplay? = null
    private var reader: ImageReader? = null
    /** The picture's pixels, reused (the share's thread only). */
    private var frameBitmap: Bitmap? = null
    private var thread: HandlerThread? = null
    private var handler: Handler? = null
    private var screenOff: BroadcastReceiver? = null
    /** The computers it's shared with (node ids). */
    private val viewers = java.util.concurrent.CopyOnWriteArraySet<String>()
    /** Until then, nobody watching yet is fine (the computer is starting). */
    private var graceUntil = 0L
    /** When one of [viewers] last watched. */
    private var lastWatched = 0L

    @Volatile
    var controlAllowed = false
        private set
    /** The picture's size in pixels (taps map from it). */
    @Volatile
    var frameSize = 0 to 0
        private set

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        when (intent?.action) {
            ACTION_STOP -> {
                stop()
                return START_NOT_STICKY
            }
            ACTION_CONTROL -> {
                setControl(intent?.getBooleanExtra(EXTRA_ON, false) == true)
                return START_NOT_STICKY
            }
        }
        if (projection != null) {
            // Started again while sharing (Android wants startForeground
            // for every start): keep the one running.
            startForeground(ScreenShare.NOTE_SHARING, ScreenShare.sharingNote(this, viewers.toSet(), controlAllowed))
            return START_NOT_STICKY
        }
        ScreenShare.channel(this)
        val node = intent?.getStringExtra(ScreenConsentActivity.EXTRA_NODE).orEmpty()
        val note = ScreenShare.sharingNote(this, setOfNotNull(node.ifEmpty { null }), false)
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
            startForeground(ScreenShare.NOTE_SHARING, note, ServiceInfo.FOREGROUND_SERVICE_TYPE_MEDIA_PROJECTION)
        } else {
            startForeground(ScreenShare.NOTE_SHARING, note)
        }
        val code = intent?.getIntExtra(EXTRA_CODE, 0) ?: 0
        @Suppress("DEPRECATION")
        val data = intent?.getParcelableExtra<Intent>(EXTRA_DATA)
        val mpm = getSystemService(MediaProjectionManager::class.java)
        val mp = if (data != null && node.isNotEmpty()) runCatching { mpm?.getMediaProjection(code, data) }.getOrNull() else null
        if (mp == null) {
            stop()
            return START_NOT_STICKY
        }
        start(mp)
        addViewer(node, intent?.getBooleanExtra(ScreenConsentActivity.EXTRA_OFFER, false) == true)
        return START_NOT_STICKY
    }

    private fun start(mp: MediaProjection) {
        projection = mp
        current = this
        ResourceLedger.hold(ResourceBook.Kind.SERVICE, "ScreenShareService")
        controlAllowed = false
        graceUntil = System.currentTimeMillis() + START_GRACE_MS
        lastWatched = 0L
        val t = HandlerThread("screen-share").also { it.start() }
        thread = t
        val h = Handler(t.looper)
        handler = h
        mp.registerCallback(object : MediaProjection.Callback() {
            override fun onStop() = stop()
        }, h)
        // Never the lock screen: stop as the screen goes off.
        screenOff = object : BroadcastReceiver() {
            override fun onReceive(c: Context?, i: Intent?) {
                Log.i(TAG, "screen off: stop sharing")
                stop()
            }
        }.also { r -> runCatching { registerReceiver(r, IntentFilter(Intent.ACTION_SCREEN_OFF)) } }
        // The whole screen (status and navigation bars too), scaled to WIDTH.
        val (rw, rh) = ScreenShare.realSize(this)
        val w = WIDTH
        val ht = (rh.toLong() * WIDTH / rw.coerceAtLeast(1)).toInt().coerceAtLeast(1)
        frameSize = w to ht
        val r = ImageReader.newInstance(w, ht, PixelFormat.RGBA_8888, 2)
        reader = r
        display = mp.createVirtualDisplay(
            "my-domain screen", w, ht, resources.displayMetrics.densityDpi * WIDTH / rw.coerceAtLeast(1),
            DisplayManager.VIRTUAL_DISPLAY_FLAG_AUTO_MIRROR, r.surface, null, h,
        )
        h.post(object : Runnable {
            override fun run() {
                if (current !== this@ScreenShareService) return
                tick(r)
                h.postDelayed(this, EVERY_MS)
            }
        })
    }

    fun isViewer(node: String) = node in viewers

    /** Share with [node] too; tell it to watch ([offer]: you chose it here). */
    fun addViewer(node: String, offer: Boolean) {
        if (node.isEmpty()) return
        viewers.add(node)
        graceUntil = maxOf(graceUntil, System.currentTimeMillis() + START_GRACE_MS)
        changed()
        if (offer) Thread { offerTo(node) }.start()
    }

    /** Tell [node] to watch; with no link yet, connect and try again for a
     *  while (the share stops by itself if it never watches). */
    private fun offerTo(node: String) {
        val e = Core.engine() ?: return
        repeat(OFFER_TRIES) { i ->
            if (current !== this || node !in viewers) return
            val sent = runCatching { e.screenOffer(node, true) }.onFailure { Log.w(TAG, "offer: $it") }.isSuccess
            if (sent) return
            if (i == 0) runCatching { e.connect(node) }
            Thread.sleep(OFFER_RETRY_MS)
        }
    }

    /** [node] stopped watching, or you stopped sharing with it ([tell]
     *  it); with nobody left, stop. */
    fun viewerLeft(node: String, tell: Boolean = false) {
        if (!viewers.remove(node)) return
        if (tell) offerOff(listOf(node))
        if (viewers.isEmpty()) stop() else changed()
    }

    fun setControl(on: Boolean) {
        val was = controlAllowed
        // Taps need the helper; without it there's nothing to allow.
        controlAllowed = on && PhoneHelper.instance != null
        changed()
        if (was && !controlAllowed) SecureMode.controlEnded(this, wasAllowed = true)
    }

    /** The notification, the device screen and the status follow. */
    private fun changed() {
        if (current !== this) return
        runCatching {
            getSystemService(NotificationManager::class.java)
                ?.notify(ScreenShare.NOTE_SHARING, ScreenShare.sharingNote(this, viewers.toSet(), controlAllowed))
        }
        ScreenShare.published(viewers.toSet(), controlAllowed)
    }

    /** Send the latest picture to the computers shared with, if they
     *  watch; stop when none has for a while, or the phone locked. */
    private fun tick(r: ImageReader) {
        val engine = Core.engine() ?: return stop()
        if (getSystemService(KeyguardManager::class.java)?.isKeyguardLocked == true) {
            Log.i(TAG, "locked: stop sharing")
            return stop()
        }
        val now = System.currentTimeMillis()
        val to = viewers.toList()
        val watching = engine.screenWatchers().any { it in viewers }
        if (!watching) {
            r.acquireLatestImage()?.close()
            if (now > graceUntil && now - lastWatched > IDLE_MS) stop()
            return
        }
        lastWatched = now
        val image = r.acquireLatestImage() ?: return
        val jpeg = runCatching {
            val plane = image.planes[0]
            val padded = plane.rowStride / plane.pixelStride
            // One bitmap for every picture (two a second, ~4 MB each), not a new one each time.
            val bmp = frameBitmap?.takeIf { !it.isRecycled && it.width == padded && it.height == image.height }
                ?: Bitmap.createBitmap(padded, image.height, Bitmap.Config.ARGB_8888).also {
                    frameBitmap?.recycle()
                    frameBitmap = it
                }
            bmp.copyPixelsFromBuffer(plane.buffer)
            val cropped = if (padded != image.width) Bitmap.createBitmap(bmp, 0, 0, image.width, image.height) else bmp
            try {
                ByteArrayOutputStream().also { cropped.compress(Bitmap.CompressFormat.JPEG, QUALITY, it) }.toByteArray()
            } finally {
                if (cropped !== bmp) cropped.recycle()
            }
        }.getOrNull()
        image.close()
        if (jpeg != null) runCatching { engine.screenFrameTo(jpeg, to) }
    }

    /** Tell computers that understand it that the sharing stopped. */
    private fun offerOff(nodes: List<String>) {
        val tell = nodes.filter { Core.deviceStatus[it]?.screenView == true }
        if (tell.isEmpty()) return
        Thread {
            val e = Core.engine() ?: return@Thread
            tell.forEach { runCatching { e.screenOffer(it, false) } }
        }.start()
    }

    private fun stop() {
        val wasCurrent = current === this
        if (wasCurrent) current = null
        offerOff(viewers.toList())
        viewers.clear()
        if (controlAllowed) SecureMode.controlEnded(this, wasAllowed = true)
        controlAllowed = false
        screenOff?.let { runCatching { unregisterReceiver(it) } }
        screenOff = null
        runCatching { display?.release() }
        runCatching { reader?.close() }
        runCatching { projection?.stop() }
        display = null
        reader = null
        projection = null
        // On the share's thread, after any picture being made.
        val h = handler
        if (h != null) h.post { frameBitmap?.recycle(); frameBitmap = null }
        handler = null
        thread?.quitSafely()
        thread = null
        ResourceLedger.release(ResourceBook.Kind.SERVICE, "ScreenShareService")
        if (wasCurrent) ScreenShare.published(emptySet(), false)
        stopForeground(STOP_FOREGROUND_REMOVE)
        stopSelf()
    }

    override fun onDestroy() {
        if (projection != null || current === this) stop()
        super.onDestroy()
    }

    companion object {
        private const val TAG = "mydomain_screen"
        const val EXTRA_CODE = "code"
        const val EXTRA_DATA = "data"
        const val EXTRA_ON = "on"
        const val ACTION_STOP = "stop"
        const val ACTION_CONTROL = "control"
        /** Snapshot width in pixels. */
        private const val WIDTH = 720
        private const val QUALITY = 50
        private const val EVERY_MS = 500L
        /** Time for a computer to start watching after the share starts
         *  (it hears the offer, or Android's prompt was answered). */
        private const val START_GRACE_MS = 20_000L
        /** Stop this long after the computers stopped watching (a watch
         *  that isn't renewed runs out after 25 s: a lost link ends here). */
        private const val IDLE_MS = 5_000L
        private const val OFFER_TRIES = 8
        private const val OFFER_RETRY_MS = 1_500L

        /** The sharing running now, if any. */
        @Volatile
        var current: ScreenShareService? = null
            private set

        val running: Boolean get() = current != null
    }
}
