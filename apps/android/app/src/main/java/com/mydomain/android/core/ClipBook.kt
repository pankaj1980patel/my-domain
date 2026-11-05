package com.mydomain.android.core

/** A kept picture: its files in `clip-images` ([file].png, and the
 *  thumbnail [file].thumb.jpg), its size in pixels, and the PNG's size on
 *  disk. */
data class ClipPicture(val file: String, val width: Int, val height: Int, val bytes: Long)

/** One clip as kept: text this phone sent to [node]'s clipboard
 *  ([incoming] false) or [node] sent to this phone's ([incoming] true), or a
 *  copied picture ([picture]; then [text] names it, for search).
 *  [at]: milliseconds since 1970. */
data class ClipRecord(
    val id: Long,
    val text: String,
    val truncated: Boolean,
    val node: String,
    val incoming: Boolean,
    val at: Long,
    val picture: ClipPicture? = null,
)

/** What a device's clipboard history tile shows: the first matches, how
 *  many match, and how many that device has kept in all. */
data class ClipPage(val entries: List<ClipRecord>, val total: Int, val kept: Int)

/**
 * The clipboard history's list and its rules, without Android (so JVM tests
 * can check it); [ClipLog] keeps one and saves it. The same as the desktop's
 * `clip_history.rs`: the newest [MAX_ENTRIES] clips (all devices together),
 * newest first, each one's text kept up to [MAX_TEXT] bytes (UTF-8) and
 * marked `truncated` when it was cut. Pictures among them: the newest
 * [MAX_PICTURES], no more than [MAX_PICTURE_BYTES] in all; the ones that
 * fall out are handed over by [takeDropped] so their files can go.
 */
class ClipBook(list: List<ClipRecord> = emptyList()) {
    /** Newest first. */
    var list: List<ClipRecord> = list.take(MAX_ENTRIES)
        private set

    private var nextId: Long = (this.list.maxOfOrNull { it.id } ?: 0L) + 1

    /** Pictures that fell out since [takeDropped] was last called. */
    private val dropped = mutableListOf<ClipPicture>()

    init {
        this.list = keep(this.list)
    }

    /** Keep a clip; none for blank text. The one kept, if any. */
    fun add(node: String, incoming: Boolean, text: String, at: Long): ClipRecord? {
        if (text.isBlank() || node.isEmpty()) return null
        val (kept, cut) = capText(text, MAX_TEXT)
        val e = ClipRecord(nextId++, kept, cut, node, incoming, at)
        list = keep(record(list, e, MAX_ENTRIES))
        return e
    }

    /** Keep a picture clip; none when it alone is over the bytes cap (its
     *  files are then the caller's to delete). The one kept, if any. */
    fun addPicture(node: String, incoming: Boolean, picture: ClipPicture, at: Long): ClipRecord? {
        if (node.isEmpty() || picture.bytes > MAX_PICTURE_BYTES) return null
        val e = ClipRecord(nextId++, pictureText(picture.width, picture.height), false, node, incoming, at, picture)
        list = keep(record(list, e, MAX_ENTRIES))
        return e
    }

    /** Clips read back from the file, after the ones kept since (they're
     *  newer). Ids the newer ones share with older ones are given new ones. */
    fun addOlder(older: List<ClipRecord>) {
        val oldIds = older.mapTo(HashSet()) { it.id }
        var next = maxOf(nextId, (older.maxOfOrNull { it.id } ?: 0L) + 1)
        val newer = list.map { if (it.id in oldIds) it.copy(id = next++) else it }
        list = keep((newer + older).take(MAX_ENTRIES), extra = older)
        nextId = maxOf(next, (list.maxOfOrNull { it.id } ?: 0L) + 1)
    }

    /** Forget [node]'s clips. Whether there were any. */
    fun clear(node: String): Boolean {
        val next = list.filter { it.node != node }
        if (next.size == list.size) return false
        list = keep(next)
        return true
    }

    fun clearAll() {
        list = keep(emptyList())
    }

    /** The pictures that fell out since the last call (their files can go). */
    fun takeDropped(): List<ClipPicture> {
        val out = dropped.toList()
        dropped.clear()
        return out
    }

    /** [next] within the picture caps, noting every picture of the current
     *  list (and of [extra]) that isn't in it any more. */
    private fun keep(next: List<ClipRecord>, extra: List<ClipRecord> = emptyList()): List<ClipRecord> {
        val capped = capPictures(next, MAX_PICTURES, MAX_PICTURE_BYTES)
        val still = capped.mapNotNullTo(HashSet()) { it.picture?.file }
        (list + extra).mapNotNull { it.picture }.filter { it.file !in still }.distinctBy { it.file }.forEach {
            if (dropped.none { d -> d.file == it.file }) dropped.add(it)
        }
        return capped
    }

    /** [node]'s clips (either way) matching [query], newest first: the first [limit]. */
    fun page(node: String, query: String, limit: Int): ClipPage = page(list, node, query, limit)

    fun encode(): String = encode(list)

    companion object {
        /** The most clips kept. */
        const val MAX_ENTRIES = 200
        /** The most text kept of one clip, in bytes (UTF-8). */
        const val MAX_TEXT = 10 * 1024
        /** The most pictures kept. */
        const val MAX_PICTURES = 50
        /** The most bytes of pictures kept, all together. */
        const val MAX_PICTURE_BYTES = 100L * 1024 * 1024
        /** A thumbnail's longer side, at most, in pixels. */
        const val THUMB_MAX = 256

        /** The text a picture's clip is kept with: what search finds it by
         *  ("picture", "image"). The same as the desktop's. */
        fun pictureText(width: Int, height: Int): String = "Picture (image), $width × $height"

        /** [list] (newest first) without the pictures past the caps: beyond
         *  the newest [maxCount], or once together they'd be over [maxBytes]. */
        fun capPictures(list: List<ClipRecord>, maxCount: Int, maxBytes: Long): List<ClipRecord> {
            var count = 0
            var bytes = 0L
            return list.filter { e ->
                val p = e.picture ?: return@filter true
                count++
                bytes += p.bytes
                count <= maxCount && bytes <= maxBytes
            }
        }

        /** A thumbnail's size for a [width] x [height] picture: the longer
         *  side at most [max] (never enlarged), at least 1 px each way. */
        fun thumbSize(width: Int, height: Int, max: Int = THUMB_MAX): Pair<Int, Int> {
            val long = maxOf(width, height)
            if (long <= max || long <= 0) return width to height
            fun scale(v: Int) = ((v.toLong() * max + long / 2) / long).toInt().coerceAtLeast(1)
            return scale(width) to scale(height)
        }

        /** The power-of-two step to decode a [width] x [height] picture at
         *  so it's still at least [max] px along its longer side. */
        fun sampleSize(width: Int, height: Int, max: Int): Int {
            var s = 1
            while (maxOf(width, height) / (s * 2) >= max) s *= 2
            return s
        }

        /** A file name made here: letters, digits, `-` and `_` only. */
        fun safeFile(name: String): Boolean =
            name.isNotEmpty() && name.length <= 64 && name.all { it.isLetterOrDigit() || it == '-' || it == '_' }

        /** [text] cut to at most [maxBytes] bytes of UTF-8, never through a
         *  character (nor between the halves of a surrogate pair), and
         *  whether it was cut. */
        fun capText(text: String, maxBytes: Int): Pair<String, Boolean> {
            var bytes = 0
            var i = 0
            while (i < text.length) {
                val c = text[i]
                val pair = c.isHighSurrogate() && i + 1 < text.length && text[i + 1].isLowSurrogate()
                val size = when {
                    pair -> 4
                    c.code < 0x80 -> 1
                    c.code < 0x800 -> 2
                    else -> 3
                }
                if (bytes + size > maxBytes) return text.substring(0, i) to true
                bytes += size
                i += if (pair) 2 else 1
            }
            return text to false
        }

        /** [node]'s clips in [list] (either way) matching [query], newest
         *  first: the first [limit]. */
        fun page(list: List<ClipRecord>, node: String, query: String, limit: Int): ClipPage {
            val mine = list.filter { it.node == node }
            val hits = mine.filter { matches(it.text, query) }
            return ClipPage(hits.take(limit.coerceIn(0, MAX_ENTRIES)), hits.size, mine.size)
        }

        /** [entry] first in [list], dropping the oldest past [max]. */
        fun record(list: List<ClipRecord>, entry: ClipRecord, max: Int): List<ClipRecord> =
            (listOf(entry) + list).take(max)

        /** Every word of [query] appears in [text], ignoring case. An empty
         *  query matches everything. */
        fun matches(text: String, query: String): Boolean {
            val hay = text.lowercase()
            return query.lowercase().split(Regex("\\s+")).filter { it.isNotEmpty() }.all { hay.contains(it) }
        }

        /** The file's text: a JSON array of
         *  `{"id", "text", "truncated", "node", "direction": "in"/"out", "at"}`,
         *  plus `"picture": {"file", "width", "height", "bytes"}` for a picture. */
        fun encode(list: List<ClipRecord>): String = Json.write(
            list.map {
                linkedMapOf<String, Any?>(
                    "id" to it.id,
                    "text" to it.text,
                    "truncated" to it.truncated,
                    "node" to it.node,
                    "direction" to if (it.incoming) "in" else "out",
                    "at" to it.at,
                ).apply {
                    val p = it.picture ?: return@apply
                    put("picture", linkedMapOf("file" to p.file, "width" to p.width, "height" to p.height, "bytes" to p.bytes))
                }
            },
        )

        /** The clips in the file's [text], newest first; ones that don't make
         *  sense are left out, and unreadable text is none. */
        fun decode(text: String): List<ClipRecord> {
            val items = runCatching { Json.parse(text) }.getOrNull() as? List<*> ?: return emptyList()
            return items.mapNotNull { item ->
                val o = item as? Map<*, *> ?: return@mapNotNull null
                val id = (o["id"] as? Number)?.toLong() ?: return@mapNotNull null
                val body = o["text"] as? String ?: return@mapNotNull null
                val node = o["node"] as? String ?: return@mapNotNull null
                val incoming = when (o["direction"]) {
                    "in" -> true
                    "out" -> false
                    else -> return@mapNotNull null
                }
                if (body.isBlank() || node.isEmpty()) return@mapNotNull null
                val picture = o["picture"]?.let { decodePicture(it) ?: return@mapNotNull null }
                val (kept, cut) = capText(body, MAX_TEXT)
                ClipRecord(id, kept, cut || o["truncated"] == true, node, incoming, (o["at"] as? Number)?.toLong() ?: 0L, picture)
            }.distinctBy { it.id }.take(MAX_ENTRIES)
        }

        private fun decodePicture(v: Any): ClipPicture? {
            val p = v as? Map<*, *> ?: return null
            val file = (p["file"] as? String)?.takeIf { safeFile(it) } ?: return null
            val w = (p["width"] as? Number)?.toInt() ?: return null
            val h = (p["height"] as? Number)?.toInt() ?: return null
            val bytes = (p["bytes"] as? Number)?.toLong() ?: return null
            if (w <= 0 || h <= 0 || bytes < 0) return null
            return ClipPicture(file, w, h, bytes)
        }
    }
}
