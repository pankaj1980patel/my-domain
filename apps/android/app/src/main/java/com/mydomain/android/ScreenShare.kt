package com.mydomain.android

import android.app.Activity
import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
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
import android.util.Log
import androidx.core.app.NotificationCompat
import com.mydomain.android.core.Core
import java.io.ByteArrayOutputStream

/**
 * Your computers see this phone's screen (mdcore `features::screen`, the
 * other way round from the touchpad's screen view): when a computer starts
 * watching, the phone asks you (Android's own "start recording?" prompt,
 * every time: nothing is shared without it), then sends a small JPEG of the
 * screen about twice a second until nobody watches.
 *
 * On screen, the prompt opens straight away; otherwise a notification
 * opens it when tapped (Android doesn't let apps start screens from the
 * background).
 */
object ScreenShare {
    private const val TAG = "mydomain_screen"
    private const val CH = "screen_share"
    const val NOTE_ASK = 7301
    const val NOTE_SHARING = 7302

    /** A computer started ([on]) or stopped watching this screen. */
    fun onWatch(ctx: Context, from: String, on: Boolean) {
        if (!on || ScreenShareService.running) return
        val app = ctx.applicationContext
        channel(app)
        val ask = Intent(app, ScreenConsentActivity::class.java)
            .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
            .putExtra(ScreenConsentActivity.EXTRA_FROM, Core.peerName(from))
        if (Core.visible && runCatching { app.startActivity(ask) }.isSuccess) return
        val pi = PendingIntent.getActivity(app, NOTE_ASK, ask, PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
        val n = NotificationCompat.Builder(app, CH)
            .setSmallIcon(android.R.drawable.ic_menu_view)
            .setContentTitle(app.getString(R.string.screen_ask_title, Core.peerName(from)))
            .setContentText(app.getString(R.string.screen_ask_body))
            .setPriority(NotificationCompat.PRIORITY_HIGH)
            .setAutoCancel(true)
            .setContentIntent(pi)
            .build()
        runCatching { app.getSystemService(NotificationManager::class.java)?.notify(NOTE_ASK, n) }
            .onFailure { Log.w(TAG, "can't ask: $it") }
    }

    fun channel(ctx: Context) {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.O) return
        ctx.getSystemService(NotificationManager::class.java)?.createNotificationChannel(
            NotificationChannel(CH, ctx.getString(R.string.channel_screen), NotificationManager.IMPORTANCE_HIGH),
        )
    }

    fun sharingNote(ctx: Context): Notification {
        val stop = PendingIntent.getService(
            ctx, 0, Intent(ctx, ScreenShareService::class.java).setAction(ScreenShareService.ACTION_STOP),
            PendingIntent.FLAG_IMMUTABLE,
        )
        return NotificationCompat.Builder(ctx, CH)
            .setSmallIcon(android.R.drawable.ic_menu_view)
            .setContentTitle(ctx.getString(R.string.screen_sharing_title))
            .setContentText(ctx.getString(R.string.screen_sharing_body))
            .setOngoing(true)
            .addAction(0, ctx.getString(R.string.screen_stop), stop)
            .build()
    }
}

/** Shows Android's screen-recording prompt; a yes starts the sharing. */
class ScreenConsentActivity : Activity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        getSystemService(NotificationManager::class.java)?.cancel(ScreenShare.NOTE_ASK)
        val mpm = getSystemService(MediaProjectionManager::class.java) ?: return finish()
        @Suppress("DEPRECATION")
        startActivityForResult(mpm.createScreenCaptureIntent(), ASK)
    }

    @Deprecated("Activity result API; this activity has no other result")
    override fun onActivityResult(requestCode: Int, resultCode: Int, data: Intent?) {
        @Suppress("DEPRECATION")
        super.onActivityResult(requestCode, resultCode, data)
        if (requestCode == ASK && resultCode == RESULT_OK && data != null) {
            val start = Intent(this, ScreenShareService::class.java)
                .putExtra(ScreenShareService.EXTRA_CODE, resultCode)
                .putExtra(ScreenShareService.EXTRA_DATA, data)
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) startForegroundService(start) else startService(start)
        }
        finish()
    }

    companion object {
        const val EXTRA_FROM = "from"
        private const val ASK = 1
    }
}

/** Takes the snapshots and sends them while anyone watches. */
class ScreenShareService : Service() {
    private var projection: MediaProjection? = null
    private var display: VirtualDisplay? = null
    private var reader: ImageReader? = null
    private var thread: HandlerThread? = null
    private var handler: Handler? = null
    /** When someone last watched (the core's watch list, checked each tick). */
    private var lastWatched = 0L

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        if (intent?.action == ACTION_STOP) {
            stop()
            return START_NOT_STICKY
        }
        ScreenShare.channel(this)
        val note = ScreenShare.sharingNote(this)
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
            startForeground(ScreenShare.NOTE_SHARING, note, ServiceInfo.FOREGROUND_SERVICE_TYPE_MEDIA_PROJECTION)
        } else {
            startForeground(ScreenShare.NOTE_SHARING, note)
        }
        val code = intent?.getIntExtra(EXTRA_CODE, 0) ?: 0
        @Suppress("DEPRECATION")
        val data = intent?.getParcelableExtra<Intent>(EXTRA_DATA)
        val mpm = getSystemService(MediaProjectionManager::class.java)
        val mp = if (data != null) runCatching { mpm?.getMediaProjection(code, data) }.getOrNull() else null
        if (mp == null) {
            stop()
            return START_NOT_STICKY
        }
        start(mp)
        return START_NOT_STICKY
    }

    private fun start(mp: MediaProjection) {
        projection = mp
        running = true
        lastWatched = System.currentTimeMillis()
        val t = HandlerThread("screen-share").also { it.start() }
        thread = t
        val h = Handler(t.looper)
        handler = h
        mp.registerCallback(object : MediaProjection.Callback() {
            override fun onStop() = stop()
        }, h)
        val metrics = resources.displayMetrics
        val w = WIDTH
        val ht = (metrics.heightPixels.toLong() * WIDTH / metrics.widthPixels).toInt()
        val r = ImageReader.newInstance(w, ht, PixelFormat.RGBA_8888, 2)
        reader = r
        display = mp.createVirtualDisplay(
            "my-domain screen", w, ht, metrics.densityDpi * WIDTH / metrics.widthPixels,
            DisplayManager.VIRTUAL_DISPLAY_FLAG_AUTO_MIRROR, r.surface, null, h,
        )
        h.post(object : Runnable {
            override fun run() {
                if (!running) return
                tick(r)
                h.postDelayed(this, EVERY_MS)
            }
        })
    }

    /** Send the latest picture, if anyone watches; stop when nobody has for a while. */
    private fun tick(r: ImageReader) {
        val engine = Core.engine() ?: return stop()
        if (engine.screenWatchers().isEmpty()) {
            if (System.currentTimeMillis() - lastWatched > IDLE_MS) stop()
            r.acquireLatestImage()?.close()
            return
        }
        lastWatched = System.currentTimeMillis()
        val image = r.acquireLatestImage() ?: return
        val jpeg = runCatching {
            val plane = image.planes[0]
            val padded = plane.rowStride / plane.pixelStride
            val bmp = Bitmap.createBitmap(padded, image.height, Bitmap.Config.ARGB_8888)
            bmp.copyPixelsFromBuffer(plane.buffer)
            val cropped = if (padded != image.width) Bitmap.createBitmap(bmp, 0, 0, image.width, image.height) else bmp
            ByteArrayOutputStream().also { cropped.compress(Bitmap.CompressFormat.JPEG, QUALITY, it) }.toByteArray()
        }.getOrNull()
        image.close()
        if (jpeg != null) runCatching { engine.screenFrame(jpeg) }
    }

    private fun stop() {
        running = false
        runCatching { display?.release() }
        runCatching { reader?.close() }
        runCatching { projection?.stop() }
        display = null
        reader = null
        projection = null
        thread?.quitSafely()
        thread = null
        stopForeground(STOP_FOREGROUND_REMOVE)
        stopSelf()
    }

    override fun onDestroy() {
        if (running) stop()
        super.onDestroy()
    }

    companion object {
        const val EXTRA_CODE = "code"
        const val EXTRA_DATA = "data"
        const val ACTION_STOP = "stop"
        /** Snapshot width in pixels. */
        private const val WIDTH = 720
        private const val QUALITY = 50
        private const val EVERY_MS = 500L
        /** Stop sharing after this long with nobody watching. */
        private const val IDLE_MS = 30_000L

        @Volatile
        var running = false
            private set
    }
}
