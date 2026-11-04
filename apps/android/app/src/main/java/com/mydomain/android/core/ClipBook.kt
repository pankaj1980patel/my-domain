package com.mydomain.android.core

/** One clip as kept: text this phone sent to [node]'s clipboard
 *  ([incoming] false) or [node] sent to this phone's ([incoming] true).
 *  [at]: milliseconds since 1970. */
data class ClipRecord(
    val id: Long,
    val text: String,
    val truncated: Boolean,
    val node: String,
    val incoming: Boolean,
    val at: Long,
)

/** What a device's clipboard history tile shows: the first matches, how
 *  many match, and how many that device has kept in all. */
data class ClipPage(val entries: List<ClipRecord>, val total: Int, val kept: Int)

/**
 * The clipboard history's list and its rules, without Android (so JVM tests
 * can check it); [ClipLog] keeps one and saves it. The same as the desktop's
 * `clip_history.rs`: the newest [MAX_ENTRIES] clips (all devices together),
 * newest first, each one's text kept up to [MAX_TEXT] bytes (UTF-8) and
 * marked `truncated` when it was cut.
 */
class ClipBook(list: List<ClipRecord> = emptyList()) {
    /** Newest first. */
    var list: List<ClipRecord> = list.take(MAX_ENTRIES)
        private set

    private var nextId: Long = (this.list.maxOfOrNull { it.id } ?: 0L) + 1

    /** Keep a clip; none for blank text. The one kept, if any. */
    fun add(node: String, incoming: Boolean, text: String, at: Long): ClipRecord? {
        if (text.isBlank() || node.isEmpty()) return null
        val (kept, cut) = capText(text, MAX_TEXT)
        val e = ClipRecord(nextId++, kept, cut, node, incoming, at)
        list = record(list, e, MAX_ENTRIES)
        return e
    }

    /** Clips read back from the file, after the ones kept since (they're
     *  newer). Ids the newer ones share with older ones are given new ones. */
    fun addOlder(older: List<ClipRecord>) {
        val oldIds = older.mapTo(HashSet()) { it.id }
        var next = maxOf(nextId, (older.maxOfOrNull { it.id } ?: 0L) + 1)
        val newer = list.map { if (it.id in oldIds) it.copy(id = next++) else it }
        list = (newer + older).take(MAX_ENTRIES)
        nextId = maxOf(next, (list.maxOfOrNull { it.id } ?: 0L) + 1)
    }

    /** Forget [node]'s clips. Whether there were any. */
    fun clear(node: String): Boolean {
        val next = list.filter { it.node != node }
        if (next.size == list.size) return false
        list = next
        return true
    }

    fun clearAll() {
        list = emptyList()
    }

    /** [node]'s clips (either way) matching [query], newest first: the first [limit]. */
    fun page(node: String, query: String, limit: Int): ClipPage = page(list, node, query, limit)

    fun encode(): String = encode(list)

    companion object {
        /** The most clips kept. */
        const val MAX_ENTRIES = 200
        /** The most text kept of one clip, in bytes (UTF-8). */
        const val MAX_TEXT = 10 * 1024

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
         *  `{"id", "text", "truncated", "node", "direction": "in"/"out", "at"}`. */
        fun encode(list: List<ClipRecord>): String = Json.write(
            list.map {
                linkedMapOf(
                    "id" to it.id,
                    "text" to it.text,
                    "truncated" to it.truncated,
                    "node" to it.node,
                    "direction" to if (it.incoming) "in" else "out",
                    "at" to it.at,
                )
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
                val (kept, cut) = capText(body, MAX_TEXT)
                ClipRecord(id, kept, cut || o["truncated"] == true, node, incoming, (o["at"] as? Number)?.toLong() ?: 0L)
            }.distinctBy { it.id }.take(MAX_ENTRIES)
        }
    }
}
