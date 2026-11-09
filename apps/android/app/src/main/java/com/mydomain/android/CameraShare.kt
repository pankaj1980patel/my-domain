package com.mydomain.android

import com.mydomain.android.core.NotificationIds
import android.Manifest
import android.annotation.SuppressLint
import android.app.Activity
import android.app.AlertDialog
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
import android.content.pm.PackageManager
import android.content.pm.ServiceInfo
import android.graphics.ImageFormat
import android.graphics.Rect
import android.graphics.YuvImage
import android.hardware.camera2.CameraCaptureSession
import android.hardware.camera2.CameraCharacteristics
import android.hardware.camera2.CameraDevice
import android.hardware.camera2.CameraManager
import android.hardware.camera2.CaptureRequest
import android.media.ImageReader
import android.os.Build
import android.os.Bundle
import android.os.Handler
import android.os.HandlerThread
import android.os.IBinder
import android.util.Log
import android.view.OrientationEventListener
import android.widget.Toast
import androidx.core.app.NotificationCompat
import com.mydomain.android.core.CameraFrames
import com.mydomain.android.core.Core
import com.mydomain.android.core.OutgoingFiles
import java.io.ByteArrayOutputStream
import android.util.Range
import com.mydomain.android.core.ResourceBook
import com.mydomain.android.core.ResourceLedger

/**
 * Your computers see what this phone's camera sees (mdcore
 * `features::camera`): look at something across the room, or use the phone
 * as a document camera, and take a photo from the computer.
 *
 *  - A computer asks (`camera_watch`): the phone asks you, "Show your camera
 *    to …?" (a prompt on screen, else a notification), every time. Nothing
 *    is captured until you tap Allow; the first time, Android asks for the
 *    camera permission too (after saying why).
 *  - While it shows, a notification says "Your camera is on for …" with
 *    Stop. It stops on Stop, when the computer stops asking or its link
 *    goes, and when the phone locks or its screen goes off (unless "Keep
 *    streaming while locked" is on in Settings, off by default).
 *  - The computer may switch between the back and front cameras, turn the
 *    torch on and off, and take a photo: a full-size JPEG sent to it as an
 *    ordinary file. Pictures go only to the computers you allowed, about
 *    12 a second, never kept on the phone.
 */
object CameraShare {
    private const val TAG = "mydomain_camera"
    private const val CH = "camera_share"
    const val NOTE_ASK = NotificationIds.CAMERA_ASK
    const val NOTE_ON = NotificationIds.CAMERA_ON
    /** "Keep streaming while locked" (Settings): off by default. */
    const val PREF_KEEP_LOCKED = "camera_keep_locked"

    fun keepWhileLocked(ctx: Context): Boolean =
        ctx.getSharedPreferences("md", Context.MODE_PRIVATE).getBoolean(PREF_KEEP_LOCKED, false)

    fun setKeepWhileLocked(ctx: Context, on: Boolean) {
        ctx.getSharedPreferences("md", Context.MODE_PRIVATE).edit().putBoolean(PREF_KEEP_LOCKED, on).apply()
    }

    /** A computer started ([on]) or stopped asking to see this camera. */
    fun onWatch(ctx: Context, from: String, on: Boolean) {
        val app = ctx.applicationContext
        if (!on) {
            CameraShareService.current?.viewerLeft(from)
            return
        }
        val svc = CameraShareService.current
        if (svc != null && svc.isViewer(from)) return
        Log.i(TAG, "asked to show the camera (app on screen: ${Core.visible}, on: ${svc != null})")
        channel(app)
        val ask = Intent(app, CameraConsentActivity::class.java)
            .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
            .putExtra(CameraConsentActivity.EXTRA_NODE, from)
        // Already showing it to another computer: ask by notification only.
        if (svc == null && Core.visible && runCatching { app.startActivity(ask) }.isSuccess) return
        val pi = PendingIntent.getActivity(app, NOTE_ASK, ask, PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
        val n = NotificationCompat.Builder(app, CH)
            .setSmallIcon(android.R.drawable.ic_menu_camera)
            .setContentTitle(app.getString(R.string.camera_ask_title, Core.peerName(from)))
            .setContentText(app.getString(R.string.camera_ask_body))
            .setPriority(NotificationCompat.PRIORITY_HIGH)
            .setAutoCancel(true)
            .setContentIntent(pi)
            .build()
        runCatching { app.getSystemService(NotificationManager::class.java)?.notify(NOTE_ASK, n) }
            .onFailure { Log.w(TAG, "can't ask: $it") }
    }

    /** The computer watching asks the camera to [op] ("switch", "torch"
     *  with [on], "photo"). Only from a computer it's shown to. */
    fun onControl(from: String, op: String, on: Boolean) {
        val svc = CameraShareService.current ?: return
        if (!svc.isViewer(from)) return
        when (op) {
            "switch" -> svc.switchCamera()
            "torch" -> svc.setTorch(on)
            "photo" -> svc.takePhoto(from)
        }
    }

    /** The link to [node] went: stop showing it the camera. */
    fun linkLost(node: String) {
        CameraShareService.current?.viewerLeft(node)
    }

    /** Tell [node] the camera is off for it, and why ("declined", ...). */
    fun tellOff(node: String, reason: String) {
        Thread {
            runCatching { Core.engine()?.cameraOffer(node, false, false, null, reason) }
                .onFailure { Log.w(TAG, "can't say the camera is off: $it") }
        }.start()
    }

    fun channel(ctx: Context) {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.O) return
        ctx.getSystemService(NotificationManager::class.java)?.createNotificationChannel(
            NotificationChannel(CH, ctx.getString(R.string.channel_camera), NotificationManager.IMPORTANCE_HIGH),
        )
    }

    /** "Your camera is on for …", with Stop. */
    fun onNote(ctx: Context, viewers: Set<String>): Notification {
        val stop = PendingIntent.getService(
            ctx, 0, Intent(ctx, CameraShareService::class.java).setAction(CameraShareService.ACTION_STOP),
            PendingIntent.FLAG_IMMUTABLE,
        )
        val names = viewers.joinToString(", ") { Core.peerName(it) }
        return NotificationCompat.Builder(ctx, CH)
            .setSmallIcon(android.R.drawable.ic_menu_camera)
            .setContentTitle(
                if (names.isEmpty()) ctx.getString(R.string.camera_on_title_plain)
                else ctx.getString(R.string.camera_on_title, names),
            )
            .setContentText(ctx.getString(R.string.camera_on_body))
            .setOngoing(true)
            .setOnlyAlertOnce(true)
            .addAction(0, ctx.getString(R.string.camera_stop), stop)
            .build()
    }
}

/** "Show your camera to …?": Allow starts it (asking for the camera
 *  permission first if it's not granted yet); anything else says no. */
class CameraConsentActivity : Activity() {
    private var node = ""
    private var answered = false

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        getSystemService(NotificationManager::class.java)?.cancel(CameraShare.NOTE_ASK)
        node = intent?.getStringExtra(EXTRA_NODE).orEmpty()
        if (node.isEmpty()) return finish()
        CameraShareService.current?.let { svc -> if (svc.isViewer(node)) return finish() }
        val name = Core.peerName(node)
        val body = getString(R.string.camera_ask_dialog_body, name) +
            if (hasCamera()) "" else "\n\n" + getString(R.string.camera_permission_why)
        AlertDialog.Builder(this, android.R.style.Theme_DeviceDefault_Dialog_Alert)
            .setTitle(getString(R.string.camera_ask_title, name))
            .setMessage(body)
            .setPositiveButton(R.string.camera_allow) { _, _ -> allow() }
            .setNegativeButton(R.string.camera_deny) { _, _ -> refuse("declined", null) }
            .setOnCancelListener { refuse("declined", null) }
            .show()
    }

    private fun hasCamera() =
        checkSelfPermission(Manifest.permission.CAMERA) == PackageManager.PERMISSION_GRANTED

    private fun allow() {
        answered = true
        if (hasCamera()) start() else requestPermissions(arrayOf(Manifest.permission.CAMERA), ASK_CAMERA)
    }

    override fun onRequestPermissionsResult(requestCode: Int, permissions: Array<out String>, grantResults: IntArray) {
        super.onRequestPermissionsResult(requestCode, permissions, grantResults)
        if (requestCode != ASK_CAMERA) return
        if (grantResults.firstOrNull() == PackageManager.PERMISSION_GRANTED) start()
        else refuse("no_permission", R.string.camera_permission_denied)
    }

    private fun start() {
        CameraShareService.current?.let { svc ->
            svc.addViewer(node)
            return finish()
        }
        val i = Intent(this, CameraShareService::class.java).putExtra(EXTRA_NODE, node)
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) startForegroundService(i) else startService(i)
        finish()
    }

    private fun refuse(reason: String, toast: Int?) {
        if (answered && reason == "declined") return
        answered = true
        toast?.let { Toast.makeText(this, getString(it), Toast.LENGTH_LONG).show() }
        CameraShare.tellOff(node, reason)
        finish()
    }

    companion object {
        /** The computer asking (its node id). */
        const val EXTRA_NODE = "node"
        private const val ASK_CAMERA = 1
    }
}

/** Opens the camera (Camera2) and sends its pictures to the computers you
 *  allowed; takes full-size photos for them. Everything camera runs on
 *  its own thread ([handler]). */
class CameraShareService : Service() {
    private var thread: HandlerThread? = null
    private var handler: Handler? = null
    private var device: CameraDevice? = null
    private var session: CameraCaptureSession? = null
    private var preview: ImageReader? = null
    private var still: ImageReader? = null
    private var orientation: OrientationEventListener? = null
    private var screenOff: BroadcastReceiver? = null
    /** The computers it's shown to (node ids). */
    private val viewers = java.util.concurrent.CopyOnWriteArraySet<String>()
    /** Who asked for the photo being taken. */
    private val photoFor = java.util.concurrent.CopyOnWriteArraySet<String>()
    @Volatile private var front = false
    @Volatile private var torch = false
    @Volatile private var hasFlash = false
    @Volatile private var sensorDeg = 90
    /** The preview's frame rate range ([CameraFrames.pickFpsRange]). */
    @Volatile private var fpsRange: Pair<Int, Int>? = null
    /** How the phone is held, clockwise (OrientationEventListener). */
    @Volatile private var held = 0
    private var lastSent = 0L
    /** Until then, nobody watching yet is fine (the computer is starting). */
    private var graceUntil = 0L
    private var lastWatched = 0L
    private var stopping = false

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        if (intent?.action == ACTION_STOP) {
            stop("stopped")
            return START_NOT_STICKY
        }
        val node = intent?.getStringExtra(CameraConsentActivity.EXTRA_NODE).orEmpty()
        CameraShare.channel(this)
        val note = CameraShare.onNote(this, viewers.toSet() + setOfNotNull(node.ifEmpty { null }))
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
            startForeground(CameraShare.NOTE_ON, note, ServiceInfo.FOREGROUND_SERVICE_TYPE_CAMERA)
        } else {
            startForeground(CameraShare.NOTE_ON, note)
        }
        if (current === this) {
            // Started again while on: one more computer.
            addViewer(node)
            return START_NOT_STICKY
        }
        if (node.isEmpty()) {
            stop(null)
            return START_NOT_STICKY
        }
        current = this
        stopping = false
        ResourceLedger.hold(ResourceBook.Kind.CAMERA, "camera share")
        val t = HandlerThread("camera-share").also { it.start() }
        thread = t
        val h = Handler(t.looper)
        handler = h
        graceUntil = System.currentTimeMillis() + START_GRACE_MS
        viewers.add(node)
        orientation = object : OrientationEventListener(this) {
            override fun onOrientationChanged(deg: Int) {
                held = CameraFrames.quarterTurn(if (deg == OrientationEventListener.ORIENTATION_UNKNOWN) -1 else deg, held)
            }
        }.also { if (it.canDetectOrientation()) it.enable() }
        // Never from a locked phone, unless you said so.
        if (!CameraShare.keepWhileLocked(this)) {
            screenOff = object : BroadcastReceiver() {
                override fun onReceive(c: Context?, i: Intent?) {
                    Log.i(TAG, "screen off: camera off")
                    stop("locked")
                }
            }.also { r -> runCatching { registerReceiver(r, IntentFilter(Intent.ACTION_SCREEN_OFF)) } }
        }
        h.post { open() }
        return START_NOT_STICKY
    }

    fun isViewer(node: String) = node in viewers

    /** Show the camera to [node] too. */
    fun addViewer(node: String) {
        if (node.isEmpty()) return
        viewers.add(node)
        graceUntil = maxOf(graceUntil, System.currentTimeMillis() + START_GRACE_MS)
        changed()
        handler?.post { announce(listOf(node)) }
    }

    /** [node] stopped asking (or its link went): with nobody left, stop. */
    fun viewerLeft(node: String) {
        if (!viewers.remove(node)) return
        photoFor.remove(node)
        if (viewers.isEmpty()) stop(null) else changed()
    }

    fun switchCamera() {
        handler?.post {
            front = !front
            torch = false
            closeCamera()
            open()
        }
    }

    fun setTorch(on: Boolean) {
        handler?.post {
            if (!hasFlash) return@post
            torch = on
            repeat()
            announce(viewers.toList())
        }
    }

    /** A full-size photo for [node], sent as a file once taken. */
    fun takePhoto(node: String) {
        handler?.post {
            val d = device ?: return@post
            val s = session ?: return@post
            val target = still ?: return@post
            photoFor.add(node)
            runCatching {
                val b = d.createCaptureRequest(CameraDevice.TEMPLATE_STILL_CAPTURE)
                b.addTarget(target.surface)
                b.set(CaptureRequest.CONTROL_AF_MODE, CaptureRequest.CONTROL_AF_MODE_CONTINUOUS_PICTURE)
                b.set(CaptureRequest.CONTROL_AE_MODE, CaptureRequest.CONTROL_AE_MODE_ON)
                b.set(CaptureRequest.FLASH_MODE, if (torch) CaptureRequest.FLASH_MODE_TORCH else CaptureRequest.FLASH_MODE_OFF)
                b.set(CaptureRequest.JPEG_ORIENTATION, CameraFrames.rotation(sensorDeg, held, front))
                b.set(CaptureRequest.JPEG_QUALITY, PHOTO_QUALITY)
                s.capture(b.build(), null, handler)
            }.onFailure {
                Log.w(TAG, "photo: $it")
                photoFor.remove(node)
            }
        }
    }

    // --- the camera (on the handler thread) ---------------------------------

    // The permission was granted before this service started (the consent
    // activity asks); one taken back since lands in the SecurityException.
    @SuppressLint("MissingPermission")
    private fun open() {
        val cm = getSystemService(CameraManager::class.java) ?: return stop("error")
        val ids = runCatching { cm.cameraIdList.toList() }.getOrElse { return stop("error") }
        val want = if (front) CameraCharacteristics.LENS_FACING_FRONT else CameraCharacteristics.LENS_FACING_BACK
        val id = ids.firstOrNull { runCatching { cm.getCameraCharacteristics(it).get(CameraCharacteristics.LENS_FACING) }.getOrNull() == want }
            ?: ids.firstOrNull()
            ?: return stop("error")
        val chars = runCatching { cm.getCameraCharacteristics(id) }.getOrElse { return stop("error") }
        front = chars.get(CameraCharacteristics.LENS_FACING) == CameraCharacteristics.LENS_FACING_FRONT
        sensorDeg = chars.get(CameraCharacteristics.SENSOR_ORIENTATION) ?: 90
        hasFlash = chars.get(CameraCharacteristics.FLASH_INFO_AVAILABLE) == true
        // About a dozen pictures a second go out: no need for the camera to make 30.
        fpsRange = CameraFrames.pickFpsRange(
            chars.get(CameraCharacteristics.CONTROL_AE_AVAILABLE_TARGET_FPS_RANGES).orEmpty().map { it.lower to it.upper },
        )
        if (!hasFlash) torch = false
        val map = chars.get(CameraCharacteristics.SCALER_STREAM_CONFIGURATION_MAP) ?: return stop("error")
        val pick = CameraFrames.pickSize(map.getOutputSizes(ImageFormat.YUV_420_888).orEmpty().map { it.width to it.height })
            ?: return stop("error")
        val big = map.getOutputSizes(ImageFormat.JPEG).orEmpty().maxByOrNull { it.width.toLong() * it.height }
            ?: return stop("error")
        val h = handler ?: return
        preview = ImageReader.newInstance(pick.first, pick.second, ImageFormat.YUV_420_888, 3).also { r ->
            r.setOnImageAvailableListener({ onPreview(it) }, h)
        }
        still = ImageReader.newInstance(big.width, big.height, ImageFormat.JPEG, 2).also { r ->
            r.setOnImageAvailableListener({ onStill(it) }, h)
        }
        try {
            cm.openCamera(id, object : CameraDevice.StateCallback() {
                override fun onOpened(d: CameraDevice) {
                    if (current !== this@CameraShareService) return d.close()
                    device = d
                    configure(d)
                }

                override fun onDisconnected(d: CameraDevice) {
                    d.close()
                    if (device === d) stop("busy")
                }

                override fun onError(d: CameraDevice, error: Int) {
                    d.close()
                    Log.w(TAG, "camera error $error")
                    val busy = error == CameraDevice.StateCallback.ERROR_CAMERA_IN_USE ||
                        error == CameraDevice.StateCallback.ERROR_MAX_CAMERAS_IN_USE ||
                        error == CameraDevice.StateCallback.ERROR_CAMERA_DISABLED
                    stop(if (busy) "busy" else "error")
                }
            }, h)
        } catch (e: SecurityException) {
            stop("no_permission")
        } catch (e: Exception) {
            Log.w(TAG, "can't open the camera: $e")
            stop("busy")
        }
    }

    private fun configure(d: CameraDevice) {
        val p = preview ?: return
        val s = still ?: return
        try {
            @Suppress("DEPRECATION")
            d.createCaptureSession(listOf(p.surface, s.surface), object : CameraCaptureSession.StateCallback() {
                override fun onConfigured(cs: CameraCaptureSession) {
                    if (device !== d) return cs.close()
                    session = cs
                    repeat()
                    announce(viewers.toList())
                }

                override fun onConfigureFailed(cs: CameraCaptureSession) = stop("error")
            }, handler)
        } catch (e: Exception) {
            Log.w(TAG, "session: $e")
            stop("error")
        }
    }

    /** The preview, with the torch as it's set. */
    private fun repeat() {
        val d = device ?: return
        val s = session ?: return
        val p = preview ?: return
        runCatching {
            val b = d.createCaptureRequest(CameraDevice.TEMPLATE_PREVIEW)
            b.addTarget(p.surface)
            b.set(CaptureRequest.CONTROL_AF_MODE, CaptureRequest.CONTROL_AF_MODE_CONTINUOUS_PICTURE)
            b.set(CaptureRequest.CONTROL_AE_MODE, CaptureRequest.CONTROL_AE_MODE_ON)
            b.set(CaptureRequest.FLASH_MODE, if (torch) CaptureRequest.FLASH_MODE_TORCH else CaptureRequest.FLASH_MODE_OFF)
            fpsRange?.let { (lo, hi) -> b.set(CaptureRequest.CONTROL_AE_TARGET_FPS_RANGE, Range(lo, hi)) }
            s.setRepeatingRequest(b.build(), null, handler)
        }.onFailure { Log.w(TAG, "preview: $it") }
    }

    /** Tell [nodes] the camera is on, which one, and the torch. */
    private fun announce(nodes: List<String>) {
        if (session == null) return
        val e = Core.engine() ?: return
        val t: Boolean? = if (hasFlash) torch else null
        nodes.forEach { n -> runCatching { e.cameraOffer(n, true, front, t, null) }.onFailure { Log.w(TAG, "offer: $it") } }
    }

    /** A preview picture: to the computers watching, a dozen a second;
     *  stop when none has for a while, or the phone locked. */
    private fun onPreview(r: ImageReader) {
        val image = r.acquireLatestImage() ?: return
        try {
            // Most frames end here: the checks below (the lock screen, who
            // watches) are calls of their own, made only for a frame that goes.
            val now = System.currentTimeMillis()
            if (now - lastSent < EVERY_MS) return
            lastSent = now
            val engine = Core.engine() ?: return stop("error")
            if (!CameraShare.keepWhileLocked(this) && getSystemService(KeyguardManager::class.java)?.isKeyguardLocked == true) {
                Log.i(TAG, "locked: camera off")
                return stop("locked")
            }
            val to = engine.cameraWatchers().filter { it in viewers }
            if (to.isEmpty()) {
                if (now > graceUntil && now - lastWatched > IDLE_MS) stop(null)
                return
            }
            lastWatched = now
            val (w, h) = image.width to image.height
            val pl = image.planes
            val nv21 = CameraFrames.toNv21(
                w, h, pl[0].buffer, pl[0].rowStride,
                pl[1].buffer, pl[2].buffer, pl[1].rowStride, pl[1].pixelStride,
            )
            val deg = CameraFrames.rotation(sensorDeg, held, front)
            val turned = CameraFrames.rotateNv21(nv21, w, h, deg)
            val (tw, th) = CameraFrames.rotatedSize(w, h, deg)
            val out = ByteArrayOutputStream()
            YuvImage(turned, ImageFormat.NV21, tw, th, null).compressToJpeg(Rect(0, 0, tw, th), QUALITY, out)
            runCatching { engine.cameraFrameTo(out.toByteArray(), to) }
        } catch (e: Exception) {
            Log.w(TAG, "frame: $e")
        } finally {
            image.close()
        }
    }

    /** A photo: written to the cache and sent to whoever asked. */
    private fun onStill(r: ImageReader) {
        val image = r.acquireNextImage() ?: return
        val bytes = try {
            val buf = image.planes[0].buffer
            ByteArray(buf.remaining()).also { buf.get(it) }
        } catch (e: Exception) {
            Log.w(TAG, "photo bytes: $e")
            null
        } finally {
            image.close()
        }
        val to = photoFor.toList().filter { it in viewers }
        photoFor.clear()
        if (bytes == null || to.isEmpty()) return
        val name = CameraFrames.photoName(System.currentTimeMillis())
        Thread {
            val e = Core.engine() ?: return@Thread
            for (node in to) {
                val file = OutgoingFiles.newFile(this, name)
                try {
                    file.writeBytes(bytes)
                    OutgoingFiles.track(e.sendFile(node, file.path), file)
                } catch (ex: Exception) {
                    Log.w(TAG, "photo not sent: $ex")
                    OutgoingFiles.discard(file)
                }
            }
        }.start()
    }

    private fun closeCamera() {
        runCatching { session?.close() }
        runCatching { device?.close() }
        runCatching { preview?.close() }
        runCatching { still?.close() }
        session = null
        device = null
        preview = null
        still = null
    }

    /** The notification follows who it's shown to. */
    private fun changed() {
        if (current !== this) return
        runCatching {
            getSystemService(NotificationManager::class.java)?.notify(CameraShare.NOTE_ON, CameraShare.onNote(this, viewers.toSet()))
        }
    }

    /** Camera off; the computers still shown to hear [reason] (none: they
     *  stopped asking themselves). Safe from any thread, more than once. */
    private fun stop(reason: String?) {
        synchronized(this) {
            if (stopping) return
            stopping = true
        }
        if (current === this) current = null
        val left = viewers.toList()
        viewers.clear()
        photoFor.clear()
        if (reason != null) left.forEach { CameraShare.tellOff(it, reason) }
        screenOff?.let { runCatching { unregisterReceiver(it) } }
        screenOff = null
        orientation?.disable()
        orientation = null
        val h = handler
        val t = thread
        handler = null
        thread = null
        if (h != null && t != null) {
            h.post {
                closeCamera()
                t.quitSafely()
            }
        } else {
            closeCamera()
        }
        ResourceLedger.release(ResourceBook.Kind.CAMERA, "camera share")
        stopForeground(STOP_FOREGROUND_REMOVE)
        stopSelf()
    }

    override fun onDestroy() {
        if (current === this) stop("stopped")
        super.onDestroy()
    }

    companion object {
        private const val TAG = "mydomain_camera"
        const val ACTION_STOP = "stop"
        private const val QUALITY = 60
        private const val PHOTO_QUALITY: Byte = 92
        /** About 12 pictures a second. */
        private const val EVERY_MS = 80L
        /** Time for the computer to (re)start watching after you allowed it. */
        private const val START_GRACE_MS = 20_000L
        /** Stop this long after the computers stopped watching (a watch that
         *  isn't renewed runs out after 25 s: a lost link ends here too). */
        private const val IDLE_MS = 5_000L

        /** The camera shown now, if any. */
        @Volatile
        var current: CameraShareService? = null
            private set
    }
}
