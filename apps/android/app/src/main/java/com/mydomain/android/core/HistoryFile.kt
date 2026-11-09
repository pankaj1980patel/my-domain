package com.mydomain.android.core

import java.io.File

/**
 * Reading a saved history (chat, clipboard, transfers, activity) safely.
 *
 * A file that can't be read as what it should be used to load as an empty
 * history, and the next save then wrote that over it: every message lost.
 * Now such a file is moved aside first, to `<name>.corrupt` (or
 * `<name>.corrupt-2`, `-3`... so an earlier one is never overwritten),
 * where it can still be looked at or recovered, and that is logged.
 */
object HistoryFile {
    /** At most this many corrupt copies are kept per file. */
    const val MAX_CORRUPT = 5

    /**
     * The text of [f] when it holds a history ([valid] says), null when
     * there is no file (or it's empty). An invalid file is moved aside and
     * [log] told where; null then too, and the history starts empty.
     */
    fun read(f: File, valid: (String) -> Boolean, log: (String) -> Unit): String? {
        if (!f.exists()) return null
        val text = runCatching { f.readText() }.getOrNull()
        if (text != null && f.length() == 0L) return null
        if (text != null && runCatching { valid(text) }.getOrDefault(false)) return text
        val aside = quarantine(f)
        log(
            if (aside != null) "${f.name} is corrupt: moved to ${aside.name}, starting empty"
            else "${f.name} is corrupt and couldn't be moved aside",
        )
        return null
    }

    /** Move [f] to the first free `.corrupt` name; null when it couldn't. */
    fun quarantine(f: File): File? {
        val names = listOf("${f.name}.corrupt") + (2..MAX_CORRUPT).map { "${f.name}.corrupt-$it" }
        val target = names.map { File(f.parentFile, it) }.firstOrNull { !it.exists() }
            // All taken: the oldest copy (the plain .corrupt) is kept; the newest slot is reused.
            ?: File(f.parentFile, names.last())
        if (target.exists()) target.delete()
        if (f.renameTo(target)) return target
        return runCatching { f.copyTo(target, overwrite = true); f.delete(); target }.getOrNull()
    }

    /** [text] parses as JSON whose top level is a list. */
    fun isJsonList(text: String): Boolean = Json.parse(text) is List<*>

    /** [text] parses as JSON whose top level is an object. */
    fun isJsonObject(text: String): Boolean = Json.parse(text) is Map<*, *>
}
