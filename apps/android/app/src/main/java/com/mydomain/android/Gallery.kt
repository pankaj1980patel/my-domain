package com.mydomain.android

import android.Manifest
import android.content.ContentResolver
import android.content.ContentUris
import android.content.Context
import android.content.pm.PackageManager
import android.database.Cursor
import android.graphics.Bitmap
import android.net.Uri
import android.os.Build
import android.os.Bundle
import android.provider.MediaStore
import android.util.Log
import android.util.Size
import androidx.core.content.ContextCompat
import com.mydomain.android.core.Core
import com.mydomain.android.ffi.GalleryMedia
import org.json.JSONObject
import java.io.ByteArrayOutputStream

/**
 * The phone's photos and videos, for your computers (core `gallery-request`
 * events): newest first, a page at a time, each with a 160 px JPEG
 * thumbnail made here. The core keeps the file paths and sends a file only
 * when a computer asks for one it was shown. [PhotoBackup] reads the same
 * MediaStore rows.
 */
object Gallery {
    private const val TAG = "mydomain_gallery"
    private const val THUMB_PX = 160
    /** The core drops thumbnails bigger than this. */
    private const val MAX_THUMB = 48 * 1024

    /** Photo/video access granted (READ_MEDIA_IMAGES / _VIDEO on Android 13+,
     *  storage before). */
    fun canRead(ctx: Context): Boolean {
        val granted = { p: String -> ContextCompat.checkSelfPermission(ctx, p) == PackageManager.PERMISSION_GRANTED }
        return if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            granted(Manifest.permission.READ_MEDIA_IMAGES) || granted(Manifest.permission.READ_MEDIA_VIDEO)
        } else {
            granted(Manifest.permission.READ_EXTERNAL_STORAGE)
        }
    }

    /** The permissions to ask for photo access. */
    fun permissions(): Array<String> =
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            arrayOf(Manifest.permission.READ_MEDIA_IMAGES, Manifest.permission.READ_MEDIA_VIDEO)
        } else {
            arrayOf(Manifest.permission.READ_EXTERNAL_STORAGE)
        }

    /** One MediaStore row. `taken`/`added` are unix seconds. */
    data class Media(
        val id: Long, val name: String, val mime: String, val size: Long,
        val taken: Long, val added: Long, val width: Int, val height: Int,
        val durationMs: Long, val video: Boolean, val path: String,
    ) {
        val uri: Uri get() = ContentUris.withAppendedId(
            if (video) MediaStore.Video.Media.EXTERNAL_CONTENT_URI else MediaStore.Images.Media.EXTERNAL_CONTENT_URI, id,
        )
    }

    private val FILES: Uri = MediaStore.Files.getContentUri("external")
    private const val TYPE = MediaStore.Files.FileColumns.MEDIA_TYPE
    private const val ONLY_MEDIA = "$TYPE IN (${MediaStore.Files.FileColumns.MEDIA_TYPE_IMAGE}, ${MediaStore.Files.FileColumns.MEDIA_TYPE_VIDEO})"

    // Column names as strings: some moved between MediaColumns and the
    // image/video columns across API levels, the names stayed the same.
    private val COLUMNS = arrayOf(
        MediaStore.Files.FileColumns._ID, "_display_name", "mime_type", "_size",
        "datetaken", "date_added", "width", "height", "duration", TYPE, "_data",
    )

    /**
     * Photos and videos matching [selection], ordered by date added
     * ([newestFirst] or oldest first), [limit] from [offset].
     */
    fun query(
        ctx: Context, selection: String? = null, args: Array<String>? = null,
        newestFirst: Boolean = true, limit: Int, offset: Int = 0,
    ): List<Media> {
        val where = if (selection == null) ONLY_MEDIA else "$ONLY_MEDIA AND ($selection)"
        val order = "date_added ${if (newestFirst) "DESC" else "ASC"}, ${MediaStore.Files.FileColumns._ID} ${if (newestFirst) "DESC" else "ASC"}"
        val cr = ctx.contentResolver
        val cursor: Cursor? = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            val q = Bundle().apply {
                putString(ContentResolver.QUERY_ARG_SQL_SELECTION, where)
                putStringArray(ContentResolver.QUERY_ARG_SQL_SELECTION_ARGS, args)
                putString(ContentResolver.QUERY_ARG_SQL_SORT_ORDER, order)
                putInt(ContentResolver.QUERY_ARG_LIMIT, limit)
                putInt(ContentResolver.QUERY_ARG_OFFSET, offset)
            }
            cr.query(FILES, COLUMNS, q, null)
        } else {
            cr.query(FILES, COLUMNS, where, args, "$order LIMIT $limit OFFSET $offset")
        }
        val out = ArrayList<Media>()
        cursor?.use { c ->
            val col = { name: String -> c.getColumnIndex(name) }
            val iId = col(MediaStore.Files.FileColumns._ID)
            val iName = col("_display_name"); val iMime = col("mime_type"); val iSize = col("_size")
            val iTaken = col("datetaken"); val iAdded = col("date_added")
            val iW = col("width"); val iH = col("height"); val iDur = col("duration")
            val iType = col(TYPE); val iData = col("_data")
            fun long(i: Int) = if (i >= 0 && !c.isNull(i)) c.getLong(i) else 0L
            fun str(i: Int) = if (i >= 0 && !c.isNull(i)) c.getString(i) ?: "" else ""
            while (c.moveToNext()) {
                val added = long(iAdded)
                val takenMs = long(iTaken)
                out += Media(
                    id = long(iId), name = str(iName), mime = str(iMime), size = long(iSize),
                    taken = if (takenMs > 0) takenMs / 1000 else added, added = added,
                    width = long(iW).toInt(), height = long(iH).toInt(), durationMs = long(iDur),
                    video = long(iType).toInt() == MediaStore.Files.FileColumns.MEDIA_TYPE_VIDEO,
                    path = str(iData),
                )
            }
        }
        return out
    }

    /** A small JPEG of [m] (at most [MAX_THUMB] bytes), or null. */
    private fun thumbnail(ctx: Context, m: Media): ByteArray? = runCatching {
        val bmp: Bitmap? = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
            ctx.contentResolver.loadThumbnail(m.uri, Size(THUMB_PX, THUMB_PX), null)
        } else if (m.video) {
            @Suppress("DEPRECATION")
            MediaStore.Video.Thumbnails.getThumbnail(ctx.contentResolver, m.id, MediaStore.Video.Thumbnails.MINI_KIND, null)
        } else {
            @Suppress("DEPRECATION")
            MediaStore.Images.Thumbnails.getThumbnail(ctx.contentResolver, m.id, MediaStore.Images.Thumbnails.MINI_KIND, null)
        }
        bmp ?: return@runCatching null
        val scaled = scaleDown(bmp)
        var bytes: ByteArray? = null
        for (quality in intArrayOf(75, 55, 35)) {
            val out = ByteArrayOutputStream()
            scaled.compress(Bitmap.CompressFormat.JPEG, quality, out)
            if (out.size() <= MAX_THUMB) { bytes = out.toByteArray(); break }
        }
        if (scaled !== bmp) scaled.recycle()
        bmp.recycle()
        bytes
    }.onFailure { Log.d(TAG, "no thumbnail for ${m.id}: $it") }.getOrNull()

    /** [bmp] with its longer side at most [THUMB_PX]. */
    private fun scaleDown(bmp: Bitmap): Bitmap {
        val longer = maxOf(bmp.width, bmp.height)
        if (longer <= THUMB_PX) return bmp
        val f = THUMB_PX.toFloat() / longer
        return Bitmap.createScaledBitmap(bmp, (bmp.width * f).toInt().coerceAtLeast(1), (bmp.height * f).toInt().coerceAtLeast(1), true)
    }

    /** `gallery_request`: list the page and answer the asking device. */
    fun onRequest(ctx: Context, o: JSONObject) {
        val engine = Core.engine() ?: return
        val from = o.optString("from")
        val req = o.optString("req")
        if (from.isEmpty() || req.isEmpty()) return
        if (!canRead(ctx)) {
            engine.galleryReply(from, req, emptyList(), null, ctx.getString(R.string.gallery_no_access))
            return
        }
        val offset = o.optLong("offset").coerceIn(0, Int.MAX_VALUE.toLong()).toInt()
        val limit = o.optInt("limit", 60).coerceIn(1, 100)
        val thumbs = o.optBoolean("thumbs", true)
        val page = runCatching { query(ctx, limit = limit + 1, offset = offset) }
            .onFailure { Log.w(TAG, "gallery query failed", it) }
            .getOrElse {
                engine.galleryReply(from, req, emptyList(), null, ctx.getString(R.string.gallery_failed))
                return
            }
        val items = page.take(limit).map { m ->
            GalleryMedia(
                id = m.id.toString(), name = m.name.ifEmpty { "${m.id}" }, mime = m.mime,
                size = m.size.coerceAtLeast(0).toULong(), taken = m.taken.coerceAtLeast(0).toULong(),
                width = m.width.coerceAtLeast(0).toUInt(), height = m.height.coerceAtLeast(0).toUInt(),
                duration = m.durationMs.coerceAtLeast(0).toULong(), video = m.video, path = m.path,
                thumb = if (thumbs) thumbnail(ctx, m) else null,
            )
        }
        val next = if (page.size > limit) (offset + limit).toULong() else null
        engine.galleryReply(from, req, items, next, null)
    }
}
