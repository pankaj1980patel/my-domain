package com.mydomain.android

import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.BroadcastReceiver
import android.content.ContentValues
import android.content.Context
import android.content.Intent
import android.net.Uri
import android.os.Build
import android.os.Environment
import android.provider.MediaStore
import android.util.Log
import android.webkit.MimeTypeMap
import androidx.core.app.NotificationCompat
import androidx.core.content.FileProvider
import android.Manifest
import android.content.pm.PackageManager
import androidx.core.content.ContextCompat
import com.mydomain.android.core.Core
import com.mydomain.android.ffi.SharedDir
import org.json.JSONObject
import java.io.File
import java.util.concurrent.ConcurrentHashMap

/**
 * The phone side of file transfers: where received files go, and the OS
 * notifications for them. NetService calls these from the core's `file_*`
 * events, so receiving works with the app in the background.
 *
 * The core saves into an app-private folder ([downloadDir]); each received
 * file is then moved into the public Downloads/my-domain through MediaStore
 * (Android 10+). Older Android keeps it in the app folder and shares it
 * through a FileProvider (writing to the public folder there would need the
 * storage permission).
 */
object FileActions {
    private const val TAG = "mydomain_files"
    const val CH_PROGRESS = "file_progress"
    const val CH_FILES = "files"
    const val ACTION_CANCEL = "com.mydomain.android.CANCEL_FILE"
    const val EXTRA_ID = "id"
    private const val SUBFOLDER = "my-domain"
    private const val PROGRESS_EVERY_MS = 1000L

    /** Last progress notification per transfer (throttle). */
    private val lastProgress = ConcurrentHashMap<String, Long>()
    /** Transfers that ended: late progress events must not re-post. */
    private val ended: MutableSet<String> = ConcurrentHashMap.newKeySet()

    fun createChannels(ctx: Context) {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.O) return
        val nm = ctx.getSystemService(NotificationManager::class.java) ?: return
        nm.createNotificationChannel(
            NotificationChannel(CH_PROGRESS, ctx.getString(R.string.channel_file_progress), NotificationManager.IMPORTANCE_LOW),
        )
        nm.createNotificationChannel(
            NotificationChannel(CH_FILES, ctx.getString(R.string.channel_files), NotificationManager.IMPORTANCE_DEFAULT),
        )
    }

    /** Where the core writes received files (app-private, no permission). */
    fun downloadDir(ctx: Context): File =
        (ctx.getExternalFilesDir(Environment.DIRECTORY_DOWNLOADS) ?: File(ctx.filesDir, "downloads")).apply { mkdirs() }

    // --- shared folders -----------------------------------------------------

    /**
     * The folders your other devices may browse and download from:
     *  - "my-domain": the files received from them (public Downloads/my-domain
     *    on Android 11+, where this app may read the files it saved; the app
     *    folder they stay in on Android 9 and older). Android 10 has neither
     *    (no path access to the public folder), so it shares nothing there.
     *  - "Pictures" and "Camera": only once the user has allowed photo access
     *    (READ_MEDIA_IMAGES / READ_MEDIA_VIDEO, or storage before Android 13).
     */
    fun sharedFolders(ctx: Context): List<SharedDir> = buildList {
        val sdk = Build.VERSION.SDK_INT
        if (sdk >= Build.VERSION_CODES.R) {
            val dl = File(Environment.getExternalStoragePublicDirectory(Environment.DIRECTORY_DOWNLOADS), SUBFOLDER)
            add(SharedDir(SUBFOLDER, dl.absolutePath))
        } else if (sdk < Build.VERSION_CODES.Q) {
            add(SharedDir(SUBFOLDER, downloadDir(ctx).absolutePath))
        }
        if (canReadMedia(ctx)) {
            add(SharedDir("Pictures", Environment.getExternalStoragePublicDirectory(Environment.DIRECTORY_PICTURES).absolutePath))
            add(SharedDir("Camera", File(Environment.getExternalStoragePublicDirectory(Environment.DIRECTORY_DCIM), "Camera").absolutePath))
        }
    }

    private fun canReadMedia(ctx: Context): Boolean {
        val granted = { p: String -> ContextCompat.checkSelfPermission(ctx, p) == PackageManager.PERMISSION_GRANTED }
        return if (Sdk.rules(Build.VERSION_CODES.TIRAMISU)) {
            granted(Manifest.permission.READ_MEDIA_IMAGES) || granted(Manifest.permission.READ_MEDIA_VIDEO)
        } else {
            granted(Manifest.permission.READ_EXTERNAL_STORAGE)
        }
    }

    /** Hand [sharedFolders] to the core (at start, and when the app comes
     *  back to the front: a permission may have been granted meanwhile). */
    fun shareFolders(ctx: Context) {
        val engine = Core.engine() ?: return
        runCatching { engine.setSharedFolders(sharedFolders(ctx)) }
            .onFailure { Log.w(TAG, "can't set shared folders: $it") }
    }

    private fun notifId(id: String) = 0x4000_0000 or (id.hashCode() and 0x0FFF_FFFF)

    private fun nm(ctx: Context) = ctx.getSystemService(NotificationManager::class.java)

    // --- events ---------------------------------------------------------------

    /** `file_progress`: a progress notification while receiving (with Cancel). */
    fun onProgress(ctx: Context, o: JSONObject) {
        if (!o.optBoolean("incoming")) return
        val id = o.optString("id")
        val now = System.currentTimeMillis()
        synchronized(this) {
            if (id in ended) return
            if (now - (lastProgress[id] ?: 0L) < PROGRESS_EVERY_MS) return
            lastProgress[id] = now
        }
        val done = o.optLong("done")
        val total = o.optLong("total")
        val pct = if (total > 0) (done * 100 / total).toInt().coerceIn(0, 100) else 0
        val cancel = PendingIntent.getBroadcast(
            ctx, notifId(id),
            Intent(ctx, FileActionReceiver::class.java).setAction(ACTION_CANCEL).putExtra(EXTRA_ID, id),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
        val n = NotificationCompat.Builder(ctx, CH_PROGRESS)
            .setSmallIcon(android.R.drawable.stat_sys_download)
            .setContentTitle(ctx.getString(R.string.file_receiving_title, o.optString("name")))
            .setContentText(ctx.getString(R.string.file_receiving_body, Core.peerName(o.optString("node_id")), pct))
            .setProgress(100, pct, total <= 0)
            .setOngoing(true)
            .setOnlyAlertOnce(true)
            .setContentIntent(openApp(ctx))
            .addAction(0, ctx.getString(R.string.cancel), cancel)
            .build()
        synchronized(this) {
            if (id in ended) return
            runCatching { nm(ctx)?.notify(notifId(id), n) }
        }
    }

    /** `file_received`: move it into Downloads, then "Saved <name> from <device>". */
    fun onReceived(ctx: Context, o: JSONObject) {
        val id = o.optString("id")
        val name = o.optString("name")
        val from = Core.peerName(o.optString("from"))
        end(ctx, id)
        val saved = save(ctx, File(o.optString("path")), name) ?: return
        val (uri, mime) = saved
        Core.fileSaved(id, uri, mime)
        // A picture copied on the other device: on this clipboard too.
        if (o.optBoolean("clipboard")) {
            runCatching {
                ctx.getSystemService(android.content.ClipboardManager::class.java)
                    ?.setPrimaryClip(android.content.ClipData.newUri(ctx.contentResolver, name, uri))
            }
        }
        val open = PendingIntent.getActivity(
            ctx, notifId(id), viewIntent(uri, mime),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
        val n = NotificationCompat.Builder(ctx, CH_FILES)
            .setSmallIcon(android.R.drawable.stat_sys_download_done)
            .setContentTitle(ctx.getString(R.string.file_saved_title, name, from))
            .setContentText(ctx.getString(R.string.file_saved_body))
            .setAutoCancel(true)
            .setContentIntent(open)
            .build()
        runCatching { nm(ctx)?.notify(notifId(id), n) }
    }

    /** `file_failed`: drop the progress notification; say so unless cancelled. */
    fun onFailed(ctx: Context, o: JSONObject) {
        if (!o.optBoolean("incoming")) return
        val id = o.optString("id")
        end(ctx, id)
        val reason = o.optString("reason")
        if (reason == "cancelled") return
        val n = NotificationCompat.Builder(ctx, CH_FILES)
            .setSmallIcon(android.R.drawable.stat_notify_error)
            .setContentTitle(ctx.getString(R.string.file_failed_title, o.optString("name")))
            .setContentText(reason)
            .setAutoCancel(true)
            .setContentIntent(openApp(ctx))
            .build()
        runCatching { nm(ctx)?.notify(notifId(id), n) }
    }

    private fun end(ctx: Context, id: String) {
        synchronized(this) {
            ended.add(id)
            lastProgress.remove(id)
            runCatching { nm(ctx)?.cancel(notifId(id)) }
        }
    }

    // --- saving and opening ---------------------------------------------------

    fun mimeOf(name: String): String {
        val ext = name.substringAfterLast('.', "").lowercase()
        return MimeTypeMap.getSingleton().getMimeTypeFromExtension(ext) ?: "application/octet-stream"
    }

    /** Public Downloads/my-domain (Android 10+), else a FileProvider Uri of
     *  the app-folder copy. Returns the Uri to open and its type. */
    private fun save(ctx: Context, file: File, name: String): Pair<Uri, String>? {
        val mime = mimeOf(name)
        if (!file.isFile) {
            Log.w(TAG, "received file missing: $file")
            return null
        }
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
            val uri = runCatching { toMediaStore(ctx, file, name, mime) }
                .onFailure { Log.w(TAG, "MediaStore copy failed: $it") }.getOrNull()
            if (uri != null) {
                file.delete()
                return uri to mime
            }
        }
        return runCatching { FileProvider.getUriForFile(ctx, "${ctx.packageName}.files", file) to mime }
            .onFailure { Log.w(TAG, "can't share $file: $it") }.getOrNull()
    }

    @androidx.annotation.RequiresApi(Build.VERSION_CODES.Q)
    private fun toMediaStore(ctx: Context, file: File, name: String, mime: String): Uri {
        val cr = ctx.contentResolver
        val values = ContentValues().apply {
            put(MediaStore.Downloads.DISPLAY_NAME, name)
            put(MediaStore.Downloads.MIME_TYPE, mime)
            put(MediaStore.Downloads.RELATIVE_PATH, "${Environment.DIRECTORY_DOWNLOADS}/$SUBFOLDER")
            put(MediaStore.Downloads.IS_PENDING, 1)
        }
        val uri = cr.insert(MediaStore.Downloads.EXTERNAL_CONTENT_URI, values)
            ?: throw java.io.IOException("MediaStore insert failed")
        try {
            val out = cr.openOutputStream(uri) ?: throw java.io.IOException("can't write $uri")
            out.use { o -> file.inputStream().use { i -> i.copyTo(o, 256 * 1024) } }
            cr.update(uri, ContentValues().apply { put(MediaStore.Downloads.IS_PENDING, 0) }, null, null)
        } catch (e: Exception) {
            runCatching { cr.delete(uri, null, null) }
            throw e
        }
        return uri
    }

    private fun viewIntent(uri: Uri, mime: String) = Intent(Intent.ACTION_VIEW)
        .setDataAndType(uri, mime)
        .addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_ACTIVITY_NEW_TASK)

    /** Open a received file in another app. False when no app can. */
    fun open(ctx: Context, uri: String, mime: String?): Boolean {
        val u = Uri.parse(uri)
        return runCatching { ctx.startActivity(viewIntent(u, mime ?: "*/*")) }
            .onFailure { Log.w(TAG, "can't open $uri: $it") }.isSuccess
    }

    private fun openApp(ctx: Context) = PendingIntent.getActivity(
        ctx, 0,
        Intent(ctx, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_SINGLE_TOP),
        PendingIntent.FLAG_IMMUTABLE,
    )
}

/** The receiving notification's Cancel action. */
class FileActionReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        if (intent.action != FileActions.ACTION_CANCEL) return
        val id = intent.getStringExtra(FileActions.EXTRA_ID) ?: return
        Core.init(context)
        Thread { Core.engine()?.cancelFile(id) }.start()
    }
}
