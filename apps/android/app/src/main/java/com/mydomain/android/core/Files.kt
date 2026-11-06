package com.mydomain.android.core

import android.content.Context
import android.net.Uri
import android.provider.DocumentsContract
import android.provider.DocumentsContract.Document
import android.provider.OpenableColumns
import android.text.format.Formatter
import com.mydomain.android.R
import com.mydomain.android.core.Core.str
import java.io.File
import java.io.IOException
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.atomic.AtomicBoolean

/**
 * One file transfer with a device, kept current by the core's `file_*`
 * events. [incoming]: to this phone. A received file gets its [uri] (the
 * copy in the public Downloads) once NetService has moved it there.
 */
data class Transfer(
    val id: String,
    val node: String,
    val name: String,
    val incoming: Boolean,
    val done: Long = 0,
    val total: Long = 0,
    val state: TransferState = TransferState.Active,
    val reason: String? = null,
    val uri: String? = null,
    val mime: String? = null,
) {
    val fraction: Float get() = if (total > 0) (done.toFloat() / total).coerceIn(0f, 1f) else 0f
}

/**
 * Files on their way out. The core sends from a path, and a picked file is a
 * content Uri, so it is copied first to `cacheDir/outgoing/<random>/<name>`
 * (its own folder, so the core sends it under its real name). The copy is
 * deleted once the transfer ends (`file_sent` / `file_failed`).
 */
object OutgoingFiles {
    private val paths = ConcurrentHashMap<String, File>()
    private val ended: MutableSet<String> = ConcurrentHashMap.newKeySet()

    private fun root(ctx: Context) = File(ctx.cacheDir, "outgoing")

    /** Copy [uri] into the cache, under its display name. Blocking (IO). */
    fun copy(ctx: Context, uri: Uri): File {
        val cr = ctx.contentResolver
        val name = safeName(displayName(ctx, uri) ?: uri.lastPathSegment ?: "file")
        val dir = File(root(ctx), java.util.UUID.randomUUID().toString()).apply { mkdirs() }
        val out = File(dir, name)
        try {
            val input = cr.openInputStream(uri) ?: throw java.io.IOException("can't read $uri")
            input.use { i -> out.outputStream().use { o -> i.copyTo(o, 256 * 1024) } }
        } catch (e: Exception) {
            dir.deleteRecursively()
            throw e
        }
        return out
    }

    /** A new, empty file named [name] to send (a photo the camera just
     *  took): [track] it once sent, [discard] it if it isn't. */
    fun newFile(ctx: Context, name: String): File {
        val dir = File(root(ctx), java.util.UUID.randomUUID().toString()).apply { mkdirs() }
        return File(dir, safeName(name))
    }

    /** The copy [file] is being sent as transfer [id]. */
    fun track(id: String, file: File) {
        paths[id] = file
        if (id in ended) end(id) // it ended before send_file returned
    }

    /** Transfer [id] is over: delete its copy. Safe from any thread. */
    fun end(id: String) {
        ended.add(id)
        paths.remove(id)?.let { discard(it) }
    }

    fun discard(file: File) {
        runCatching { file.parentFile?.deleteRecursively() }
    }

    /** Copies left over from an earlier process (its transfers died with it). */
    fun clearAll(ctx: Context) {
        runCatching { root(ctx).deleteRecursively() }
    }

    private fun displayName(ctx: Context, uri: Uri): String? = runCatching {
        ctx.contentResolver.query(uri, arrayOf(OpenableColumns.DISPLAY_NAME), null, null, null)?.use { c ->
            if (c.moveToFirst()) c.getString(0) else null
        }
    }.getOrNull()?.takeIf { it.isNotBlank() }

    /** A plain file name: no folders, no leading dots, not too long. */
    fun safeName(raw: String): String {
        val n = raw.substringAfterLast('/').substringAfterLast('\\')
            .replace(Regex("[\\u0000-\\u001f]"), "")
            .trimStart('.').trim()
        return n.take(200).ifEmpty { "file" }
    }
}

/**
 * Folders on their way out. The core sends a folder from a path, and one
 * picked with the system's folder picker is a content tree Uri, so it is
 * copied first to `cacheDir/outgoing/folders/<random>/<name>/…` (hidden
 * files left out, as the core would), then sent from there. Sending a
 * folder gives no transfer ids, so the copies go once the core says no
 * folder is being sent any more (`foldersSending`, checked when an outgoing
 * transfer ends). Copies older than a day go anyway, and all of them when a
 * new process starts ([OutgoingFiles.clearAll] clears `outgoing`).
 */
object OutgoingFolders {
    /** Most bytes copied for one folder. */
    private const val MAX_BYTES = 2L * 1024 * 1024 * 1024
    /** Most files in one folder (the core's limit too). */
    private const val MAX_FILES = 10_000
    /** Deepest subfolder copied (as deep as the core walks). */
    private const val MAX_DEPTH = 32
    /** Room left on the phone after a copy. */
    private const val SPARE_BYTES = 200L * 1024 * 1024
    private const val DAY_MS = 24L * 60 * 60 * 1000

    /** Copies being sent: their `<random>` folders. */
    private val staged: MutableSet<File> = ConcurrentHashMap.newKeySet()
    private val checking = AtomicBoolean(false)

    private class Entry(val docId: String, val rel: String, val name: String)

    private fun root(ctx: Context) = File(File(ctx.cacheDir, "outgoing"), "folders")

    /**
     * Copy the folder [tree] into the cache, keeping its subfolders; returns
     * the copy (named like the folder). [onProgress] gets the files copied
     * so far and how many there are. Blocking (IO).
     */
    fun stage(ctx: Context, tree: Uri, onProgress: (done: Int, total: Int) -> Unit): File {
        sweepOld(ctx)
        val cr = ctx.contentResolver
        val topId = DocumentsContract.getTreeDocumentId(tree)
        val top = OutgoingFiles.safeName(name(ctx, DocumentsContract.buildDocumentUriUsingTree(tree, topId)) ?: "Folder")
        val tooBig = { IOException(str(R.string.folder_too_big, Formatter.formatShortFileSize(ctx, MAX_BYTES))) }

        // What's in it, first: the sizes say whether it fits.
        val entries = ArrayList<Entry>()
        var bytes = 0L
        fun walk(docId: String, rel: String, depth: Int) {
            val children = DocumentsContract.buildChildDocumentsUriUsingTree(tree, docId)
            val cols = arrayOf(Document.COLUMN_DOCUMENT_ID, Document.COLUMN_DISPLAY_NAME, Document.COLUMN_MIME_TYPE, Document.COLUMN_SIZE)
            val cursor = cr.query(children, cols, null, null, null) ?: throw IOException(str(R.string.folder_unreadable))
            cursor.use { c ->
                while (c.moveToNext()) {
                    val id = c.getString(0)
                    val name = c.getString(1)
                    // Hidden entries (.thumbnails, .nomedia) are left out.
                    if (id != null && name != null && !name.startsWith('.')) {
                        if (c.getString(2) == Document.MIME_TYPE_DIR) {
                            if (depth < MAX_DEPTH) walk(id, "$rel/${OutgoingFiles.safeName(name)}", depth + 1)
                        } else {
                            entries.add(Entry(id, rel, OutgoingFiles.safeName(name)))
                            bytes += if (c.isNull(3)) 0L else c.getLong(3)
                            if (entries.size > MAX_FILES) throw IOException(str(R.string.folder_too_many, MAX_FILES))
                            if (bytes > MAX_BYTES) throw tooBig()
                        }
                    }
                }
            }
        }
        walk(topId, top, 0)
        if (entries.isEmpty()) throw IOException(str(R.string.folder_empty))
        val base = File(root(ctx), java.util.UUID.randomUUID().toString()).apply { mkdirs() }
        if (base.usableSpace < bytes + SPARE_BYTES) {
            base.deleteRecursively()
            throw IOException(str(R.string.folder_no_space))
        }

        try {
            var copied = 0L
            entries.forEachIndexed { i, e ->
                onProgress(i, entries.size)
                val dir = File(base, e.rel).apply { mkdirs() }
                // One that can't be opened (gone since, no access) is left out.
                val input = runCatching {
                    cr.openInputStream(DocumentsContract.buildDocumentUriUsingTree(tree, e.docId))
                }.getOrNull()
                if (input != null) {
                    val out = unique(dir, e.name)
                    copied += input.use { s -> out.outputStream().use { o -> s.copyTo(o, 256 * 1024) } }
                    // The sizes it listed may have been wrong.
                    if (copied > MAX_BYTES) throw tooBig()
                }
            }
            onProgress(entries.size, entries.size)
        } catch (e: Exception) {
            base.deleteRecursively()
            throw e
        }
        return File(base, top)
    }

    /** The copy [folder] (from [stage]) is being sent. */
    fun track(folder: File) {
        folder.parentFile?.let { staged.add(it) }
    }

    /** Delete the copy [folder] (from [stage]), e.g. when it couldn't be sent. */
    fun discard(folder: File) {
        folder.parentFile?.let {
            staged.remove(it)
            runCatching { it.deleteRecursively() }
        }
    }

    /** An outgoing transfer ended: once no folder is being sent, delete the
     *  copies. Safe from any thread. */
    fun transferEnded() {
        if (staged.isEmpty() || !checking.compareAndSet(false, true)) return
        Thread {
            // The core counts a folder done just after it reports the file.
            Thread.sleep(1000)
            checking.set(false)
            // Taken first: a copy tracked after this is counted as going below.
            val before = staged.toList()
            val going = Core.engine()?.foldersSending() ?: return@Thread
            if (going == 0u) before.forEach {
                staged.remove(it)
                runCatching { it.deleteRecursively() }
            }
        }.start()
    }

    /** Copies from more than a day ago go, sent or not. */
    private fun sweepOld(ctx: Context) {
        val now = System.currentTimeMillis()
        root(ctx).listFiles()?.forEach {
            if (now - it.lastModified() > DAY_MS) {
                staged.remove(it)
                runCatching { it.deleteRecursively() }
            }
        }
    }

    /** [name] in [dir], numbered if two names came out the same. */
    private fun unique(dir: File, name: String): File {
        var f = File(dir, name)
        var n = 2
        val dot = name.lastIndexOf('.')
        while (f.exists()) {
            f = File(dir, if (dot > 0) "${name.substring(0, dot)} ($n)${name.substring(dot)}" else "$name ($n)")
            n++
        }
        return f
    }

    private fun name(ctx: Context, uri: Uri): String? = runCatching {
        ctx.contentResolver.query(uri, arrayOf(Document.COLUMN_DISPLAY_NAME), null, null, null)?.use { c ->
            if (c.moveToFirst()) c.getString(0) else null
        }
    }.getOrNull()?.takeIf { it.isNotBlank() }
}
