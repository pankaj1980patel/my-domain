package com.mydomain.android

import com.mydomain.android.ui.LIcon
import android.Manifest
import android.annotation.SuppressLint
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.graphics.Canvas
import android.graphics.ImageFormat
import android.graphics.Matrix
import android.graphics.Paint
import android.graphics.RectF
import android.graphics.SurfaceTexture
import android.graphics.pdf.PdfDocument
import android.hardware.camera2.CameraCaptureSession
import android.hardware.camera2.CameraCharacteristics
import android.hardware.camera2.CameraDevice
import android.hardware.camera2.CameraManager
import android.hardware.camera2.CaptureRequest
import android.media.ExifInterface
import android.media.ImageReader
import android.os.Bundle
import android.os.Handler
import android.os.HandlerThread
import android.os.Looper
import android.util.Log
import android.view.OrientationEventListener
import android.view.Surface
import android.view.TextureView
import android.view.WindowManager
import android.widget.Toast
import androidx.activity.ComponentActivity
import androidx.activity.compose.BackHandler
import androidx.activity.compose.setContent
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.gestures.detectDragGestures
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawingPadding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.Path
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.role
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.IntSize
import androidx.compose.ui.unit.dp
import androidx.compose.ui.viewinterop.AndroidView
import androidx.core.app.NotificationCompat
import androidx.lifecycle.lifecycleScope
import com.mydomain.android.core.CameraFrames
import com.mydomain.android.core.Core
import com.mydomain.android.core.OutgoingFiles
import com.mydomain.android.core.Paper
import com.mydomain.android.core.PaperFit
import com.mydomain.android.core.Pt
import com.mydomain.android.core.Quad
import com.mydomain.android.core.ScanFilter
import com.mydomain.android.core.ScanMath
import com.mydomain.android.core.ScanPages
import com.mydomain.android.core.friendlyError
import com.mydomain.android.ui.GhostButton
import com.mydomain.android.ui.MdTheme
import com.mydomain.android.ui.Muted
import com.mydomain.android.ui.PrimaryButton
import com.mydomain.android.ui.SoftButton
import com.mydomain.android.ui.T
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import java.io.ByteArrayInputStream
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale
import kotlin.math.hypot
import kotlin.math.max
import kotlin.math.min
import com.mydomain.android.core.ResourceBook
import com.mydomain.android.core.ResourceLedger

/**
 * Insert from phone (mdcore `features::insert_capture`): a computer asks
 * for a photo, or a document scan, taken with this phone, and it lands in
 * its Downloads and on its clipboard.
 *
 *  - The request (`insert_request`) never opens the camera by itself. With
 *    the app on screen, a screen opens asking first ("… asks for a photo",
 *    Open camera / Cancel); otherwise a notification does, and tapping it
 *    is the yes. Cancel (there, or on the notification) tells the computer.
 *  - Photo: take it, and it goes. Scan: take pages one by one; each page's
 *    edges are found ([ScanMath]) with corners to drag, it's straightened
 *    (Matrix.setPolyToPoly), and the pages can get a filter (original,
 *    grayscale, black and white), move and go away, before they go as one
 *    PDF (A4, or Letter where that's the paper).
 *  - The computer cancelling, or the core giving up after three minutes
 *    (`insert_ended`), closes the screen and the notification.
 *  - What's sent goes under the request's id, so the computer knows it's
 *    the answer; the copy here is deleted once the transfer ends.
 */
object InsertCapture {
    private const val TAG = "mydomain_insert"
    private const val NOTE_BASE = 7400
    const val ACTION_CANCEL = "com.mydomain.android.INSERT_CANCEL"
    const val EXTRA_ID = "id"
    const val EXTRA_FROM = "from"
    const val EXTRA_MODE = "mode"
    /** Opened by itself (the app was on screen): ask before the camera. */
    const val EXTRA_ASK = "ask"

    /** The capture screen showing now, if any. */
    @Volatile
    var screen: InsertCaptureActivity? = null

    private fun note(id: String) = NOTE_BASE + (id.hashCode() and 0x3f)

    fun intent(ctx: Context, id: String, from: String, mode: String, ask: Boolean): Intent =
        Intent(ctx, InsertCaptureActivity::class.java)
            .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
            .putExtra(EXTRA_ID, id)
            .putExtra(EXTRA_FROM, from)
            .putExtra(EXTRA_MODE, mode)
            .putExtra(EXTRA_ASK, ask)

    /** A computer asks for a photo or a scan ([mode]). */
    fun onRequest(ctx: Context, from: String, id: String, mode: String) {
        if (id.isEmpty() || mode !in setOf("photo", "scan")) return
        val app = ctx.applicationContext
        Log.i(TAG, "asked for a $mode (app on screen: ${Core.visible})")
        // On screen and not busy with another: ask right here.
        if (screen == null && Core.visible && runCatching { app.startActivity(intent(app, id, from, mode, true)) }.isSuccess) return
        CameraShare.channel(app)
        val open = PendingIntent.getActivity(
            app, note(id), intent(app, id, from, mode, false),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
        val cancel = PendingIntent.getBroadcast(
            app, note(id),
            Intent(app, InsertCancelReceiver::class.java).setAction(ACTION_CANCEL).putExtra(EXTRA_ID, id),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
        val name = Core.peerName(from)
        val n = NotificationCompat.Builder(app, "camera_share")
            .setSmallIcon(android.R.drawable.ic_menu_camera)
            .setContentTitle(
                app.getString(if (mode == "scan") R.string.insert_note_title_scan else R.string.insert_note_title_photo, name),
            )
            .setContentText(app.getString(R.string.insert_note_body))
            .setPriority(NotificationCompat.PRIORITY_HIGH)
            .setAutoCancel(true)
            .setContentIntent(open)
            .addAction(0, app.getString(R.string.insert_cancel), cancel)
            // The core gives up after three minutes; so does the notice.
            .setTimeoutAfter(3 * 60_000L)
            .build()
        runCatching { app.getSystemService(NotificationManager::class.java)?.notify(note(id), n) }
            .onFailure { Log.w(TAG, "can't ask: $it") }
    }

    /** Request [id] is over ("cancelled" by the computer, "timeout",
     *  "failed"): its notice goes, and its screen closes saying why. */
    fun onEnded(ctx: Context, node: String, id: String, reason: String) {
        dismiss(ctx, id)
        val s = screen
        if (s != null && s.requestId == id) {
            s.runOnUiThread { s.ended(reason) }
        } else if (reason == "failed") {
            val app = ctx.applicationContext
            Handler(Looper.getMainLooper()).post {
                Toast.makeText(app, app.getString(R.string.insert_ended_failed, Core.peerName(node)), Toast.LENGTH_LONG).show()
            }
        }
    }

    fun dismiss(ctx: Context, id: String) {
        runCatching { ctx.getSystemService(NotificationManager::class.java)?.cancel(note(id)) }
    }

    /** Tell the computer no ([reason]: "cancelled", "no_permission", "error"). */
    fun cancel(id: String, reason: String) {
        Thread {
            runCatching { Core.engine()?.insertCancel(id, reason) }
                .onFailure { Log.w(TAG, "can't cancel: $it") }
        }.start()
    }

    /** Whether request [id] is still waiting for its answer. */
    fun pending(id: String): Boolean =
        runCatching { Core.engine()?.insertRequests()?.any { it.id == id } }.getOrNull() ?: false

    /** "photo-20261106-143005.jpg", "scan-20261106-143005.pdf". */
    fun fileName(mode: String, epochMs: Long): String {
        val stamp = SimpleDateFormat("yyyyMMdd-HHmmss", Locale.US).format(Date(epochMs))
        return if (mode == "scan") "scan-$stamp.pdf" else "photo-$stamp.jpg"
    }
}

/** The notification's Cancel. */
class InsertCancelReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        if (intent.action != InsertCapture.ACTION_CANCEL) return
        val id = intent.getStringExtra(InsertCapture.EXTRA_ID) ?: return
        Core.init(context)
        InsertCapture.dismiss(context, id)
        InsertCapture.cancel(id, "cancelled")
    }
}

/** One straightened page and the filter it shows with ([shown]). */
class ScanPage(val key: Long, val bitmap: Bitmap, val filter: ScanFilter, val shown: Bitmap)

/** What the capture screen shows. */
private sealed class Stage {
    /** Opened by itself: "… asks for a photo", Open camera / Cancel. */
    object Ask : Stage()
    object Camera : Stage()
    /** A page just taken, its corners to adjust. */
    class Adjust(val shot: Bitmap, val quad: Quad) : Stage()
    object Pages : Stage()
    /** Busy: straightening, building the PDF, sending. */
    class Working(val text: String) : Stage()
}

/** The capture screen: the camera, the scan's pages, sending. */
class InsertCaptureActivity : ComponentActivity() {
    var requestId = ""
        private set
    private var from = ""
    private var mode = "photo"
    /** Answered (sent, cancelled, or ended by the computer): nothing more
     *  to tell it when the screen goes. */
    private var done = false
    private var opened = false
    private var stage by mutableStateOf<Stage>(Stage.Camera)
    private var pages by mutableStateOf<List<ScanPage>>(emptyList())
    private var nextKey = 0L
    private var camera: StillCamera? = null
    private var texture: SurfaceTexture? = null
    private var resumed = false

    private val askCamera = registerForActivityResult(ActivityResultContracts.RequestPermission()) { ok ->
        if (ok) {
            startCamera()
        } else {
            toast(getString(R.string.insert_permission_denied))
            finishWith("no_permission")
        }
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        requestId = intent.getStringExtra(InsertCapture.EXTRA_ID).orEmpty()
        from = intent.getStringExtra(InsertCapture.EXTRA_FROM).orEmpty()
        mode = intent.getStringExtra(InsertCapture.EXTRA_MODE) ?: "photo"
        if (requestId.isEmpty()) return finish()
        InsertCapture.dismiss(this, requestId)
        if (!InsertCapture.pending(requestId)) {
            done = true
            toast(getString(R.string.insert_ended_gone))
            return finish()
        }
        InsertCapture.screen = this
        window.addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
        val ask = intent.getBooleanExtra(InsertCapture.EXTRA_ASK, false)
        stage = if (ask) Stage.Ask else Stage.Camera
        setContent { MdTheme { Screen() } }
        if (!ask) wantCamera()
    }

    override fun onStart() {
        super.onStart()
        Core.setVisible(true)
    }

    override fun onStop() {
        super.onStop()
        Core.setVisible(false)
    }

    override fun onResume() {
        super.onResume()
        resumed = true
        startCamera()
    }

    override fun onPause() {
        resumed = false
        camera?.close()
        camera = null
        super.onPause()
    }

    override fun onDestroy() {
        if (InsertCapture.screen === this) InsertCapture.screen = null
        if (!done && !isChangingConfigurations && requestId.isNotEmpty()) InsertCapture.cancel(requestId, "cancelled")
        super.onDestroy()
    }

    /** The computer ended it, or the core gave up. */
    fun ended(reason: String) {
        if (done) return
        done = true
        val name = Core.peerName(from)
        toast(
            when (reason) {
                "timeout" -> getString(R.string.insert_ended_timeout, name)
                "failed" -> getString(R.string.insert_ended_failed, name)
                else -> getString(R.string.insert_ended_cancelled, name)
            },
        )
        finish()
    }

    private fun finishWith(reason: String) {
        if (!done) {
            done = true
            InsertCapture.cancel(requestId, reason)
        }
        finish()
    }

    private fun toast(text: String) = Toast.makeText(this, text, Toast.LENGTH_LONG).show()

    private fun hasCamera() = checkSelfPermission(Manifest.permission.CAMERA) == PackageManager.PERMISSION_GRANTED

    /** The camera stage: with the permission (asking once), open it. */
    private fun wantCamera() {
        stage = Stage.Camera
        if (hasCamera()) {
            startCamera()
        } else {
            toast(getString(R.string.insert_permission_why))
            askCamera.launch(Manifest.permission.CAMERA)
        }
    }

    /** Open the camera once all is there: on screen, the camera stage,
     *  the permission, the preview's surface. */
    private fun startCamera() {
        val t = texture ?: return
        if (!resumed || stage != Stage.Camera || camera != null || !hasCamera() || done) return
        val c = StillCamera(this) { runOnUiThread { toast(getString(R.string.insert_camera_error)); finishWith("error") } }
        camera = c
        c.open(t)
        if (!opened) {
            opened = true
            Thread { runCatching { Core.engine()?.insertOpened(requestId) } }.start()
        }
    }

    private fun stopCamera() {
        camera?.close()
        camera = null
    }

    // --- after the shutter ---------------------------------------------------

    private fun shutter() {
        val c = camera ?: return
        c.shoot { jpeg -> runOnUiThread { if (!done) shot(jpeg) } }
    }

    private fun shot(jpeg: ByteArray) {
        if (mode != "scan") return send(jpeg, null)
        stopCamera()
        stage = Stage.Working(getString(R.string.insert_working))
        lifecycleScope.launch {
            val r = withContext(Dispatchers.Default) {
                runCatching {
                    val bmp = decode(jpeg, MAX_SHOT)
                    // Looking for the page needs few pixels.
                    val s = min(1.0, SMALL.toDouble() / max(bmp.width, bmp.height))
                    val sw = max(1, (bmp.width * s).toInt())
                    val sh = max(1, (bmp.height * s).toInt())
                    val small = Bitmap.createScaledBitmap(bmp, sw, sh, true)
                    val px = IntArray(sw * sh).also { small.getPixels(it, 0, sw, 0, 0, sw, sh) }
                    if (small !== bmp) small.recycle()
                    bmp to ScanMath.detectOrFull(px, sw, sh).scaled(bmp.width.toDouble() / sw, bmp.height.toDouble() / sh)
                }
            }
            r.onSuccess { (bmp, q) -> stage = Stage.Adjust(bmp, q) }
                .onFailure { e ->
                    Log.w("mydomain_insert", "page: $e")
                    toast(friendlyError(e))
                    wantCamera()
                }
        }
    }

    /** Keep the page as adjusted: straightened, added to the scan. */
    private fun keep(shot: Bitmap, quad: Quad) {
        val q = if (ScanMath.isConvex(quad)) quad else ScanMath.order(quad.points)
        stage = Stage.Working(getString(R.string.insert_working))
        lifecycleScope.launch {
            val page = withContext(Dispatchers.Default) { runCatching { straighten(shot, q) } }
            page.onSuccess { bmp ->
                pages = pages + ScanPage(nextKey++, bmp, ScanFilter.ORIGINAL, bmp)
                stage = Stage.Pages
            }.onFailure { e ->
                toast(friendlyError(e))
                stage = Stage.Adjust(shot, quad)
            }
        }
    }

    private fun setFilter(key: Long, f: ScanFilter) {
        val p = pages.firstOrNull { it.key == key } ?: return
        if (p.filter == f) return
        lifecycleScope.launch {
            val shown = withContext(Dispatchers.Default) { filtered(p.bitmap, f) }
            pages = pages.map { if (it.key == key) ScanPage(key, it.bitmap, f, shown) else it }
        }
    }

    private fun sendPdf() {
        val list = pages
        if (list.isEmpty()) return
        stage = Stage.Working(getString(R.string.insert_sending, Core.peerName(from)))
        lifecycleScope.launch {
            val pdf = withContext(Dispatchers.IO) { runCatching { buildPdf(list) } }
            pdf.onSuccess { send(null, it) }.onFailure { e ->
                toast(getString(R.string.insert_failed, friendlyError(e)))
                stage = Stage.Pages
            }
        }
    }

    /** Send the photo ([jpeg]) or the PDF ([pdf]) under the request's id. */
    private fun send(jpeg: ByteArray?, pdf: ByteArray?) {
        stopCamera()
        val name = Core.peerName(from)
        stage = Stage.Working(getString(R.string.insert_sending, name))
        val file = OutgoingFiles.newFile(this, InsertCapture.fileName(mode, System.currentTimeMillis()))
        lifecycleScope.launch {
            val r = withContext(Dispatchers.IO) {
                runCatching {
                    file.writeBytes(jpeg ?: pdf ?: ByteArray(0))
                    val e = Core.engine() ?: error("not running")
                    e.insertSend(requestId, file.path)
                    OutgoingFiles.track(requestId, file)
                }
            }
            r.onSuccess {
                done = true
                toast(getString(R.string.insert_sent, name))
                finish()
            }.onFailure { e ->
                OutgoingFiles.discard(file)
                toast(getString(R.string.insert_failed, friendlyError(e)))
                // Still waiting: try again, or cancel.
                if (InsertCapture.pending(requestId)) {
                    if (mode == "scan") stage = Stage.Pages else wantCamera()
                } else {
                    finishWith("error")
                }
            }
        }
    }

    // --- the screen ------------------------------------------------------------

    @Composable
    private fun Screen() {
        val name = Core.peerName(from)
        Box(Modifier.fillMaxSize().background(T.bg).safeDrawingPadding()) {
            when (val s = stage) {
                Stage.Ask -> AskStage(name)
                Stage.Camera -> CameraStage()
                is Stage.Adjust -> AdjustStage(s)
                Stage.Pages -> PagesStage()
                is Stage.Working -> Column(
                    Modifier.fillMaxSize().padding(24.dp),
                    verticalArrangement = Arrangement.Center,
                    horizontalAlignment = Alignment.CenterHorizontally,
                ) {
                    CircularProgressIndicator()
                    Spacer(Modifier.height(16.dp))
                    Text(s.text, color = T.text)
                }
            }
        }
    }

    @Composable
    private fun AskStage(name: String) {
        Column(
            Modifier.fillMaxSize().padding(24.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp, Alignment.CenterVertically),
        ) {
            Text(
                stringResource(if (mode == "scan") R.string.insert_intro_scan else R.string.insert_intro_photo, name),
                style = MaterialTheme.typography.headlineSmall, fontWeight = FontWeight.Bold, color = T.text,
            )
            Muted(stringResource(R.string.insert_intro_body, name))
            PrimaryButton(stringResource(R.string.insert_open_camera), { wantCamera() }, Modifier.fillMaxWidth())
            GhostButton(stringResource(R.string.insert_cancel), { finishWith("cancelled") }, Modifier.fillMaxWidth())
        }
    }

    @Composable
    private fun CameraStage() {
        val ratio = camera?.ratio ?: (3f / 4f)
        Column(Modifier.fillMaxSize(), horizontalAlignment = Alignment.CenterHorizontally) {
            Box(Modifier.weight(1f).fillMaxWidth().background(Color.Black), contentAlignment = Alignment.Center) {
                AndroidView(
                    factory = { ctx ->
                        TextureView(ctx).apply {
                            surfaceTextureListener = object : TextureView.SurfaceTextureListener {
                                override fun onSurfaceTextureAvailable(st: SurfaceTexture, w: Int, h: Int) {
                                    texture = st
                                    startCamera()
                                }

                                override fun onSurfaceTextureSizeChanged(st: SurfaceTexture, w: Int, h: Int) {}

                                override fun onSurfaceTextureDestroyed(st: SurfaceTexture): Boolean {
                                    if (texture === st) texture = null
                                    stopCamera()
                                    return true
                                }

                                override fun onSurfaceTextureUpdated(st: SurfaceTexture) {}
                            }
                        }
                    },
                    modifier = Modifier.fillMaxWidth().aspectRatio(ratio),
                )
            }
            Row(
                Modifier.fillMaxWidth().padding(16.dp),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.SpaceBetween,
            ) {
                GhostButton(stringResource(R.string.insert_cancel), { finishWith("cancelled") }, Modifier.width(110.dp), small = true)
                val label = stringResource(if (mode == "scan") R.string.insert_capture_page else R.string.insert_take_photo)
                Box(
                    Modifier.size(72.dp).border(4.dp, T.text, CircleShape).padding(8.dp)
                        .background(T.text, CircleShape)
                        .semantics { contentDescription = label; role = Role.Button }
                        .clickable { shutter() },
                )
                if (mode == "scan" && pages.isNotEmpty()) {
                    GhostButton(
                        stringResource(R.string.insert_pages_count, pages.size), { stopCamera(); stage = Stage.Pages },
                        Modifier.width(110.dp), small = true,
                    )
                } else {
                    Spacer(Modifier.width(110.dp))
                }
            }
        }
    }

    @Composable
    private fun AdjustStage(s: Stage.Adjust) {
        var quad by remember(s) { mutableStateOf(s.quad) }
        var box by remember { mutableStateOf(IntSize.Zero) }
        BackHandler { wantCamera() }
        val bw = s.shot.width.toFloat()
        val bh = s.shot.height.toFloat()
        // The picture fitted in the box: its scale and top left.
        val scale = if (box.width == 0) 1f else min(box.width / bw, box.height / bh)
        val ox = (box.width - bw * scale) / 2
        val oy = (box.height - bh * scale) / 2
        val image = remember(s) { s.shot.asImageBitmap() }
        val accent = T.accent
        Column(Modifier.fillMaxSize()) {
            Text(
                stringResource(R.string.insert_adjust_title), color = T.text,
                modifier = Modifier.padding(16.dp), style = MaterialTheme.typography.titleMedium,
            )
            Box(
                Modifier.weight(1f).fillMaxWidth().onSizeChanged { box = it }
                    .pointerInput(s, scale, ox, oy) {
                        var dragging = -1
                        detectDragGestures(
                            onDragStart = { at ->
                                val pts = quad.points.map { Offset(ox + it.x.toFloat() * scale, oy + it.y.toFloat() * scale) }
                                val (i, d) = pts.mapIndexed { i, p -> i to hypot((p.x - at.x).toDouble(), (p.y - at.y).toDouble()) }
                                    .minByOrNull { it.second }!!
                                dragging = if (d < 64.dp.toPx()) i else -1
                            },
                            onDragEnd = { dragging = -1 },
                            onDrag = { change, delta ->
                                if (dragging < 0) return@detectDragGestures
                                change.consume()
                                val p = quad.points[dragging]
                                val nx = (p.x + delta.x / scale).coerceIn(0.0, bw.toDouble())
                                val ny = (p.y + delta.y / scale).coerceIn(0.0, bh.toDouble())
                                quad = quad.with(dragging, Pt(nx, ny))
                            },
                        )
                    },
            ) {
                Image(image, null, Modifier.fillMaxSize(), contentScale = ContentScale.Fit)
                Canvas(Modifier.fillMaxSize()) {
                    val pts = quad.points.map { Offset(ox + it.x.toFloat() * scale, oy + it.y.toFloat() * scale) }
                    val path = Path().apply {
                        moveTo(pts[0].x, pts[0].y)
                        pts.drop(1).forEach { lineTo(it.x, it.y) }
                        close()
                    }
                    drawPath(path, accent.copy(alpha = 0.18f))
                    drawPath(path, accent, style = Stroke(width = 3.dp.toPx()))
                    pts.forEach {
                        drawCircle(Color.White, 14.dp.toPx(), it)
                        drawCircle(accent, 14.dp.toPx(), it, style = Stroke(width = 3.dp.toPx()))
                    }
                }
            }
            Row(Modifier.fillMaxWidth().padding(16.dp), horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                GhostButton(stringResource(R.string.insert_retake), { wantCamera() }, Modifier.weight(1f))
                PrimaryButton(stringResource(R.string.insert_keep), { keep(s.shot, quad) }, Modifier.weight(1f))
            }
        }
    }

    @Composable
    private fun PagesStage() {
        BackHandler { wantCamera() }
        Column(Modifier.fillMaxSize()) {
            Text(
                stringResource(R.string.insert_pages_title), color = T.text, fontWeight = FontWeight.Bold,
                modifier = Modifier.padding(16.dp), style = MaterialTheme.typography.headlineSmall,
            )
            LazyColumn(Modifier.weight(1f).fillMaxWidth().padding(horizontal = 16.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
                itemsIndexed(pages, key = { _, p -> p.key }) { i, p ->
                    PageRow(i, p)
                }
            }
            Row(Modifier.fillMaxWidth().padding(16.dp), horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                GhostButton(stringResource(R.string.insert_cancel), { finishWith("cancelled") }, Modifier.weight(1f))
                SoftButton(stringResource(R.string.insert_add_page), { wantCamera() }, Modifier.weight(1f))
                PrimaryButton(stringResource(R.string.insert_send_pdf), { sendPdf() }, Modifier.weight(1f), enabled = pages.isNotEmpty())
            }
        }
    }

    @Composable
    private fun PageRow(i: Int, p: ScanPage) {
        val img = remember(p.shown) { p.shown.asImageBitmap() }
        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
            Image(img, stringResource(R.string.insert_page_n, i + 1), Modifier.size(96.dp, 128.dp).background(Color.White), contentScale = ContentScale.Fit)
            Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(6.dp)) {
                Text(stringResource(R.string.insert_page_n, i + 1), color = T.text, fontWeight = FontWeight.Bold)
                for ((f, label) in listOf(
                    ScanFilter.ORIGINAL to R.string.insert_filter_original,
                    ScanFilter.GRAY to R.string.insert_filter_gray,
                    ScanFilter.BW to R.string.insert_filter_bw,
                )) {
                    SoftButton(stringResource(label), { setFilter(p.key, f) }, Modifier.fillMaxWidth(), selected = p.filter == f)
                }
            }
            Column {
                IconButton({ pages = ScanPages.up(pages, i) }, enabled = i > 0) {
                    LIcon(R.drawable.ic_lucide_chevron_up, T.text, 24.dp, stringResource(R.string.insert_move_up))
                }
                IconButton({ pages = ScanPages.down(pages, i) }, enabled = i < pages.size - 1) {
                    LIcon(R.drawable.ic_lucide_chevron_down, T.text, 24.dp, stringResource(R.string.insert_move_down))
                }
                IconButton({ pages = ScanPages.delete(pages, i); if (pages.isEmpty()) wantCamera() }) {
                    LIcon(R.drawable.ic_lucide_trash_2, T.danger, 24.dp, stringResource(R.string.insert_delete_page))
                }
            }
        }
    }

    companion object {
        /** A page's photo is decoded this big at most (long side)... */
        private const val MAX_SHOT = 2400
        /** ...looked at this big for its edges... */
        private const val SMALL = 640
        /** ...and straightened to this at most: about 190 dpi on A4. */
        private const val MAX_PAGE = 1600

        /** A JPEG as a bitmap, upright (its EXIF turn applied), the long
         *  side at most [maxSide]. */
        fun decode(jpeg: ByteArray, maxSide: Int): Bitmap {
            val bounds = BitmapFactory.Options().apply { inJustDecodeBounds = true }
            BitmapFactory.decodeByteArray(jpeg, 0, jpeg.size, bounds)
            var sample = 1
            while (max(bounds.outWidth, bounds.outHeight) / (sample * 2) >= maxSide) sample *= 2
            val bmp = BitmapFactory.decodeByteArray(jpeg, 0, jpeg.size, BitmapFactory.Options().apply { inSampleSize = sample })
                ?: error("not a picture")
            val turn = runCatching {
                when (ExifInterface(ByteArrayInputStream(jpeg)).getAttributeInt(ExifInterface.TAG_ORIENTATION, ExifInterface.ORIENTATION_NORMAL)) {
                    ExifInterface.ORIENTATION_ROTATE_90 -> 90f
                    ExifInterface.ORIENTATION_ROTATE_180 -> 180f
                    ExifInterface.ORIENTATION_ROTATE_270 -> 270f
                    else -> 0f
                }
            }.getOrDefault(0f)
            val upright = if (turn == 0f) bmp else Bitmap.createBitmap(bmp, 0, 0, bmp.width, bmp.height, Matrix().apply { postRotate(turn) }, true)
            if (upright !== bmp) bmp.recycle()
            if (max(upright.width, upright.height) <= maxSide) return upright
            val s = maxSide.toFloat() / max(upright.width, upright.height)
            return Bitmap.createScaledBitmap(upright, (upright.width * s).toInt(), (upright.height * s).toInt(), true)
                .also { if (it !== upright) upright.recycle() }
        }

        /** The page inside [quad] of [shot], straightened. */
        fun straighten(shot: Bitmap, quad: Quad): Bitmap {
            val (w, h) = ScanMath.outputSize(quad, MAX_PAGE)
            val m = Matrix()
            val dst = floatArrayOf(0f, 0f, w.toFloat(), 0f, w.toFloat(), h.toFloat(), 0f, h.toFloat())
            check(m.setPolyToPoly(quad.toFloats(), 0, dst, 0, 4)) { "can't straighten that" }
            val out = Bitmap.createBitmap(w, h, Bitmap.Config.ARGB_8888)
            Canvas(out).apply {
                drawColor(android.graphics.Color.WHITE)
                drawBitmap(shot, m, Paint(Paint.FILTER_BITMAP_FLAG or Paint.ANTI_ALIAS_FLAG))
            }
            return out
        }

        /** [src] with filter [f] (the same bitmap for ORIGINAL). */
        fun filtered(src: Bitmap, f: ScanFilter): Bitmap {
            if (f == ScanFilter.ORIGINAL) return src
            val (w, h) = src.width to src.height
            val px = IntArray(w * h).also { src.getPixels(it, 0, w, 0, 0, w, h) }
            val out = ScanMath.applyFilter(px, w, h, f)
            return Bitmap.createBitmap(out, w, h, Bitmap.Config.ARGB_8888)
        }

        /** The pages as one PDF, each on its own sheet (A4, or Letter where
         *  that's the paper), turned for a wide page, inside the margins. */
        fun buildPdf(pages: List<ScanPage>): ByteArray {
            val paper = Paper.forCountry(Locale.getDefault().country)
            val doc = PdfDocument()
            try {
                pages.forEachIndexed { i, p ->
                    val bmp = p.shown
                    val sheet = PaperFit.pageFor(bmp.width, bmp.height, paper)
                    val at = PaperFit.fit(bmp.width, bmp.height, sheet)
                    val page = doc.startPage(PdfDocument.PageInfo.Builder(sheet.w.toInt(), sheet.h.toInt(), i + 1).create())
                    page.canvas.drawBitmap(
                        bmp, null,
                        RectF(at.x.toFloat(), at.y.toFloat(), (at.x + at.w).toFloat(), (at.y + at.h).toFloat()),
                        Paint(Paint.FILTER_BITMAP_FLAG),
                    )
                    doc.finishPage(page)
                }
                val out = java.io.ByteArrayOutputStream()
                doc.writeTo(out)
                return out.toByteArray()
            } finally {
                doc.close()
            }
        }
    }
}

/** The back camera for the capture screen (Camera2): a preview on a
 *  TextureView and full-size JPEGs. Everything camera runs on its own
 *  thread. */
private class StillCamera(private val ctx: Context, private val onError: () -> Unit) {
    private val thread = HandlerThread("insert-camera").also { it.start() }
    private val handler = Handler(thread.looper)
    private var device: CameraDevice? = null
    private var session: CameraCaptureSession? = null
    private var still: ImageReader? = null
    private var surface: Surface? = null
    private var orientation: OrientationEventListener? = null
    @Volatile private var held = 0
    @Volatile private var sensorDeg = 90
    @Volatile private var closed = false
    @Volatile private var onShot: ((ByteArray) -> Unit)? = null
    /** The preview's width over its height, held upright. */
    @Volatile var ratio: Float = 3f / 4f
        private set

    fun open(texture: SurfaceTexture) {
        ResourceLedger.hold(ResourceBook.Kind.CAMERA, "photo / scan for a computer")
        orientation = object : OrientationEventListener(ctx) {
            override fun onOrientationChanged(deg: Int) {
                held = CameraFrames.quarterTurn(if (deg == ORIENTATION_UNKNOWN) -1 else deg, held)
            }
        }.also { if (it.canDetectOrientation()) it.enable() }
        handler.post { start(texture) }
    }

    // The screen asked for the permission before opening; one taken back
    // since lands in the SecurityException.
    @SuppressLint("MissingPermission")
    private fun start(texture: SurfaceTexture) {
        if (closed) return
        try {
            val cm = ctx.getSystemService(CameraManager::class.java) ?: return fail()
            val ids = cm.cameraIdList.toList()
            val id = ids.firstOrNull {
                cm.getCameraCharacteristics(it).get(CameraCharacteristics.LENS_FACING) == CameraCharacteristics.LENS_FACING_BACK
            } ?: ids.firstOrNull() ?: return fail()
            val chars = cm.getCameraCharacteristics(id)
            sensorDeg = chars.get(CameraCharacteristics.SENSOR_ORIENTATION) ?: 90
            val map = chars.get(CameraCharacteristics.SCALER_STREAM_CONFIGURATION_MAP) ?: return fail()
            val big = map.getOutputSizes(ImageFormat.JPEG).orEmpty().maxByOrNull { it.width.toLong() * it.height } ?: return fail()
            // A preview of the photo's shape, not too big.
            val want = big.width.toFloat() / big.height
            val pv = map.getOutputSizes(SurfaceTexture::class.java).orEmpty()
                .filter { max(it.width, it.height) <= 1920 }
                .sortedByDescending { it.width.toLong() * it.height }
                .let { list -> list.firstOrNull { kotlin.math.abs(it.width.toFloat() / it.height - want) < 0.02f } ?: list.firstOrNull() }
                ?: return fail()
            ratio = min(pv.width, pv.height).toFloat() / max(pv.width, pv.height)
            texture.setDefaultBufferSize(pv.width, pv.height)
            val s = Surface(texture)
            surface = s
            still = ImageReader.newInstance(big.width, big.height, ImageFormat.JPEG, 2).also { r ->
                r.setOnImageAvailableListener({ onStill(it) }, handler)
            }
            cm.openCamera(id, object : CameraDevice.StateCallback() {
                override fun onOpened(d: CameraDevice) {
                    if (closed) return d.close()
                    device = d
                    configure(d, s)
                }

                override fun onDisconnected(d: CameraDevice) {
                    d.close()
                    if (device === d) fail()
                }

                override fun onError(d: CameraDevice, error: Int) {
                    d.close()
                    Log.w("mydomain_insert", "camera error $error")
                    fail()
                }
            }, handler)
        } catch (e: Exception) {
            Log.w("mydomain_insert", "can't open the camera: $e")
            fail()
        }
    }

    private fun configure(d: CameraDevice, preview: Surface) {
        val st = still ?: return
        try {
            @Suppress("DEPRECATION")
            d.createCaptureSession(listOf(preview, st.surface), object : CameraCaptureSession.StateCallback() {
                override fun onConfigured(cs: CameraCaptureSession) {
                    if (closed || device !== d) return cs.close()
                    session = cs
                    runCatching {
                        val b = d.createCaptureRequest(CameraDevice.TEMPLATE_PREVIEW)
                        b.addTarget(preview)
                        b.set(CaptureRequest.CONTROL_AF_MODE, CaptureRequest.CONTROL_AF_MODE_CONTINUOUS_PICTURE)
                        b.set(CaptureRequest.CONTROL_AE_MODE, CaptureRequest.CONTROL_AE_MODE_ON)
                        cs.setRepeatingRequest(b.build(), null, handler)
                    }.onFailure { fail() }
                }

                override fun onConfigureFailed(cs: CameraCaptureSession) = fail()
            }, handler)
        } catch (e: Exception) {
            Log.w("mydomain_insert", "session: $e")
            fail()
        }
    }

    /** Take a full-size photo; [done] gets its JPEG (on the camera thread). */
    fun shoot(done: (ByteArray) -> Unit) {
        handler.post {
            val d = device ?: return@post
            val s = session ?: return@post
            val st = still ?: return@post
            if (onShot != null) return@post // one at a time
            onShot = done
            runCatching {
                val b = d.createCaptureRequest(CameraDevice.TEMPLATE_STILL_CAPTURE)
                b.addTarget(st.surface)
                b.set(CaptureRequest.CONTROL_AF_MODE, CaptureRequest.CONTROL_AF_MODE_CONTINUOUS_PICTURE)
                b.set(CaptureRequest.CONTROL_AE_MODE, CaptureRequest.CONTROL_AE_MODE_ON)
                b.set(CaptureRequest.JPEG_ORIENTATION, CameraFrames.rotation(sensorDeg, held, false))
                b.set(CaptureRequest.JPEG_QUALITY, 92.toByte())
                s.capture(b.build(), null, handler)
            }.onFailure {
                Log.w("mydomain_insert", "photo: $it")
                onShot = null
            }
        }
    }

    private fun onStill(r: ImageReader) {
        val image = r.acquireNextImage() ?: return
        val bytes = try {
            val buf = image.planes[0].buffer
            ByteArray(buf.remaining()).also { buf.get(it) }
        } catch (e: Exception) {
            null
        } finally {
            image.close()
        }
        val cb = onShot
        onShot = null
        if (bytes != null && !closed) cb?.invoke(bytes)
    }

    private fun fail() {
        if (closed) return
        close()
        onError()
    }

    /** Close everything; safe more than once, from any thread. */
    fun close() {
        if (closed) return
        closed = true
        ResourceLedger.release(ResourceBook.Kind.CAMERA, "photo / scan for a computer")
        orientation?.disable()
        orientation = null
        handler.post {
            runCatching { session?.close() }
            runCatching { device?.close() }
            runCatching { still?.close() }
            runCatching { surface?.release() }
            session = null
            device = null
            still = null
            surface = null
            thread.quitSafely()
        }
    }
}
