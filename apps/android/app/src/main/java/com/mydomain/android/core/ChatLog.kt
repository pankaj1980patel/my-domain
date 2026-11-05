package com.mydomain.android.core

/** Where a message stands; [wire] is its name in the file (the same as the
 *  desktop's `chat_history.rs`). */
enum class ChatStatus(val wire: String) {
    /** Sent from here; handed to a link, not confirmed yet. */
    Sent("sent"),
    /** Sent from here with no link yet: goes out once there is one. */
    Queued("queued"),
    /** Sent from here, and the device confirmed it arrived. */
    Delivered("delivered"),
    /** Sent from here, and the send failed. */
    Failed("failed"),
    /** Another device sent it here. */
    Received("received");

    companion object {
        fun of(wire: String?): ChatStatus? = entries.firstOrNull { it.wire == wire }
    }
}

/** One message as kept: [fromNode] for one received, [toNode] for one sent.
 *  [id]: the message's id (shared with the core and the device). [at]:
 *  milliseconds since 1970. */
data class ChatEntry(
    val id: String,
    val fromNode: String?,
    val toNode: String?,
    val text: String,
    val truncated: Boolean,
    val at: Long,
    val status: ChatStatus,
) {
    val incoming: Boolean get() = status == ChatStatus.Received

    /** The other device: where it came from or went to. */
    val node: String get() = (if (incoming) fromNode else toNode) ?: ""

    /** Its bubble's id in a chat (one this session didn't show itself). */
    val bubble: Long get() = id.hashCode().toLong()
}

/** One page of a device's chat: oldest first, whether older ones are kept,
 *  and how many this device has kept in all. */
data class ChatPage(val entries: List<ChatEntry>, val more: Boolean, val kept: Int)

/**
 * The message history's store and its rules, without Android (so JVM tests
 * can check it); [ChatHistory] keeps one and saves it. The same rules as the
 * desktop's `chat_history.rs`: per device the newest [MAX_PER_DEVICE]
 * messages, the newest [MAX_TOTAL] over all devices, each one's text kept up
 * to [MAX_TEXT] bytes (UTF-8) and marked `truncated` when it was cut.
 *
 * Not thread-safe: [ChatHistory] uses it from the main thread only.
 */
class ChatLog(list: List<ChatEntry> = emptyList()) {
    /** node -> its messages, oldest first. */
    private val byNode = LinkedHashMap<String, MutableList<ChatEntry>>()

    /** How many are kept, over all devices. */
    var total = 0
        private set

    init {
        list.sortedBy { it.at }.forEach { add(it) }
    }

    /** Keep [entry] (its text capped), dropping the oldest past the caps.
     *  False (nothing kept) for blank text, no device, or an id already kept
     *  for that device. */
    fun add(entry: ChatEntry): Boolean {
        if (entry.text.isBlank() || entry.node.isEmpty()) return false
        val list = byNode.getOrPut(entry.node) { ArrayList() }
        if (list.any { it.id == entry.id }) return false
        val (text, cut) = ClipBook.capText(entry.text, MAX_TEXT)
        val e = entry.copy(text = text, truncated = entry.truncated || cut)
        // Mostly in order; one stamped earlier (a clock change) goes in its place.
        var at = list.size
        while (at > 0 && list[at - 1].at > e.at) at--
        list.add(at, e)
        total++
        if (list.size > MAX_PER_DEVICE) {
            list.removeAt(0)
            total--
        }
        while (total > MAX_TOTAL) dropOldest()
        return true
    }

    private fun dropOldest() {
        val node = byNode.entries.filter { it.value.isNotEmpty() }.minByOrNull { it.value[0].at }?.key
        if (node == null) {
            total = 0
            return
        }
        val list = byNode.getValue(node)
        list.removeAt(0)
        total--
        if (list.isEmpty()) byNode.remove(node)
    }

    /** Messages read back from the file, under the ones kept since it was
     *  read (those win on the same id). */
    fun addOlder(older: List<ChatEntry>) {
        older.sortedBy { it.at }.forEach { add(it) }
    }

    /** Message [id] (sent from here) now stands at [status]. A delivered one
     *  stays delivered and a received one never changes. Whether it changed. */
    fun setStatus(id: String, status: ChatStatus): Boolean {
        if (status == ChatStatus.Received) return false
        for (list in byNode.values) {
            val i = list.indexOfLast { it.id == id }
            if (i < 0) continue
            val e = list[i]
            if (e.status == status || e.status == ChatStatus.Delivered || e.status == ChatStatus.Received) return false
            list[i] = e.copy(status = status)
            return true
        }
        return false
    }

    /** [node]'s messages before message [before] (or the newest, without
     *  one): the last [limit], oldest first. A [before] that isn't kept
     *  (cleared since) gives nothing. */
    fun page(node: String, before: String?, limit: Int): ChatPage {
        val list = byNode[node] ?: return ChatPage(emptyList(), false, 0)
        val end = if (before == null) list.size else list.indexOfLast { it.id == before }
        if (end < 0) return ChatPage(emptyList(), false, list.size)
        val start = maxOf(0, end - limit.coerceIn(0, MAX_PAGE))
        return ChatPage(list.subList(start, end).toList(), start > 0, list.size)
    }

    /** Forget [node]'s messages. Whether there were any. */
    fun clear(node: String): Boolean {
        val gone = byNode.remove(node) ?: return false
        total -= gone.size
        return true
    }

    fun clearAll() {
        byNode.clear()
        total = 0
    }

    /** After a restart: what was waiting for a link went with the app (the
     *  core keeps unsent messages in memory only), so it's marked failed. */
    fun settleAfterRestart() {
        for (list in byNode.values) {
            list.replaceAll { if (it.status == ChatStatus.Queued) it.copy(status = ChatStatus.Failed) else it }
        }
    }

    /** A copy of every kept message, oldest first per device. Read only:
     *  what a search over this phone's messages reads. */
    fun snapshot(): List<ChatEntry> = byNode.values.flatMap { it.toList() }

    fun encode(): String = encode(snapshot())

    companion object {
        /** The most messages kept per device. */
        const val MAX_PER_DEVICE = 2_000
        /** The most messages kept over all devices. */
        const val MAX_TOTAL = 20_000
        /** The most text kept of one message, in bytes (UTF-8). */
        const val MAX_TEXT = 64 * 1024
        /** The most messages one page returns. */
        const val MAX_PAGE = 500

        /** The file's text: a JSON array of `{"id", "from_node", "to_node",
         *  "text", "truncated", "at", "status"}` (the desktop's layout). */
        fun encode(list: List<ChatEntry>): String = Json.write(
            list.map {
                linkedMapOf(
                    "id" to it.id,
                    "from_node" to it.fromNode,
                    "to_node" to it.toNode,
                    "text" to it.text,
                    "truncated" to it.truncated,
                    "at" to it.at,
                    "status" to it.status.wire,
                )
            },
        )

        /** The messages in the file's [text]; ones that don't make sense are
         *  left out, and unreadable text is none. */
        fun decode(text: String): List<ChatEntry> {
            val items = runCatching { Json.parse(text) }.getOrNull() as? List<*> ?: return emptyList()
            return items.mapNotNull { item ->
                val o = item as? Map<*, *> ?: return@mapNotNull null
                val id = o["id"] as? String ?: return@mapNotNull null
                val body = o["text"] as? String ?: return@mapNotNull null
                val status = ChatStatus.of(o["status"] as? String) ?: return@mapNotNull null
                if (id.isEmpty()) return@mapNotNull null
                ChatEntry(
                    id,
                    o["from_node"] as? String,
                    o["to_node"] as? String,
                    body,
                    o["truncated"] == true,
                    (o["at"] as? Number)?.toLong() ?: 0L,
                    status,
                )
            }
        }

        /** The history in the file's [text], ready to use: caps applied, and
         *  what was still queued marked failed. */
        fun read(text: String): ChatLog = ChatLog(decode(text)).also { it.settleAfterRestart() }
    }
}
