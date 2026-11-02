package com.mydomain.android.core

import android.content.Context
import android.net.Uri
import android.provider.OpenableColumns
import java.io.File
import java.util.concurrent.ConcurrentHashMap

enum class TransferState { Active, Done, Failed }

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
