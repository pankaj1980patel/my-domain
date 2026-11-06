package com.mydomain.android.core

import java.util.PriorityQueue

/**
 * One row of the Activity timeline (the desktop's `activity.rs`, same
 * actions): [action] says what happened (`file_sent`, `clip_in`, `msg_out`,
 * `call`, `link_up`, `note`…), [subject] the file, the clip's or message's
 * first line, the title; [detail] a call's state, a link's way, "on"/"off";
 * [target] its id in its own list (to open it); [bytes] and [via] a file's
 * size and how it went.
 */
data class ActivityRow(
    val id: String,
    val kind: String,
    val action: String,
    val node: String?,
    val ts: Long,
    val subject: String,
    val detail: String = "",
    val bytes: Long? = null,
    val via: String? = null,
    val target: String = "",
)

/** One thing the timeline's own record keeps (nothing else keeps it). */
data class ActivityEntry(
    val id: Long,
    val action: String,
    val node: String?,
    val ts: Long,
    val subject: String = "",
    val detail: String = "",
)

/** A timer as listed: when it was started, by which device. */
data class TimerSource(val id: String, val label: String, val durationMs: Long, val createdAt: Long, val by: String?)

/** What to list: one device, some kinds (all when empty), words, and where
 *  to go on from (the last row shown). */
data class ActivityQuery(
    val node: String? = null,
    val kinds: Set<String> = emptySet(),
    val text: String = "",
    val beforeTs: Long? = null,
    val beforeId: String? = null,
    val limit: Int = ActivityFeed.PAGE,
)

/** One page, and whether there's more after it. */
data class ActivityPage(val items: List<ActivityRow>, val more: Boolean)

/** Copies of what the phone keeps, for one page. [names]: device names as
 *  shown, for the words filter. */
data class ActivitySources(
    val names: Map<String, String> = emptyMap(),
    val transfers: List<TransferRecord> = emptyList(),
    val clips: List<ClipRecord> = emptyList(),
    val messages: List<ChatEntry> = emptyList(),
    val notes: List<NoteSource> = emptyList(),
    val links: List<ReadLaterLinks.Item> = emptyList(),
    val timers: List<TimerSource> = emptyList(),
    val log: List<ActivityEntry> = emptyList(),
)

/**
 * The timeline's own record and the view's settings: what no history keeps
 * (links coming and going, calls, rings, links opened, screens and sounds
 * shared, key changes, wake-ups), newest first, at most [MAX]; the kinds
 * hidden; and since when the view was cleared. Never the clipboard's: the
 * clipboard history decides what's kept ([ClipPrivacy]).
 */
class ActivityRecord(
    entries: List<ActivityEntry> = emptyList(),
    hidden: List<String> = emptyList(),
    var clearedAt: Long? = null,
) {
    var entries: List<ActivityEntry> = entries.sortedByDescending { it.ts }.take(MAX)
        private set
    var hidden: List<String> = ActivityFeed.KINDS.filter { it in hidden }
        private set
    private var nextId: Long = (this.entries.maxOfOrNull { it.id } ?: 0L)
    /** Devices linked right now (not saved): a link coming up twice is one entry. */
    private val up = HashSet<String>()

    /** Keep an entry, unless it's the clipboard's, the same as the newest
     *  within a minute, or a link's state it's already in. Whether it changed. */
    fun push(action: String, node: String?, subject: String, detail: String, ts: Long): Boolean {
        if (action.isEmpty() || ActivityFeed.kindOf(action) == "clipboard") return false
        if (node != null) {
            if (action == "link_up" && !up.add(node)) return false
            if (action == "link_down" && !up.remove(node)) return false
        }
        val last = entries.firstOrNull()
        if (last != null && last.action == action && last.node == node && last.subject == subject &&
            last.detail == detail && kotlin.math.abs(ts - last.ts) < REPEAT_MS
        ) return false
        val e = ActivityEntry(++nextId, action, node, ts, subject.take(ActivityFeed.SUBJECT * 4), detail.take(ActivityFeed.SUBJECT))
        entries = (listOf(e) + entries).take(MAX)
        return true
    }

    /** "Clear activity": the record goes, rows before [now] are hidden (the histories keep them). */
    fun clear(now: Long) {
        entries = emptyList()
        clearedAt = now
    }

    /** Show the rows from before a clear again. */
    fun restore() {
        clearedAt = null
    }

    /** Hide these kinds (unknown ones dropped, in [ActivityFeed.KINDS] order). */
    fun setHidden(kinds: Collection<String>) {
        hidden = ActivityFeed.KINDS.filter { it in kinds }
    }

    /** The file's text: `{"entries": [...], "hidden": [...], "cleared_at": n}`. */
    fun encode(): String = Json.write(
        linkedMapOf(
            "entries" to entries.map {
                linkedMapOf("id" to it.id, "action" to it.action, "node" to it.node, "ts" to it.ts, "subject" to it.subject, "detail" to it.detail)
            },
            "hidden" to hidden,
            "cleared_at" to clearedAt,
        ),
    )

    companion object {
        const val MAX = 1000
        private const val REPEAT_MS = 60_000L

        /** The record in the file's [text]; unreadable text is an empty one. */
        fun decode(text: String): ActivityRecord {
            val o = runCatching { Json.parse(text) }.getOrNull() as? Map<*, *> ?: return ActivityRecord()
            val entries = (o["entries"] as? List<*>).orEmpty().mapNotNull { item ->
                val e = item as? Map<*, *> ?: return@mapNotNull null
                val action = e["action"] as? String ?: return@mapNotNull null
                ActivityEntry(
                    (e["id"] as? Number)?.toLong() ?: 0L,
                    action,
                    e["node"] as? String,
                    (e["ts"] as? Number)?.toLong() ?: 0L,
                    e["subject"] as? String ?: "",
                    e["detail"] as? String ?: "",
                )
            }
            val hidden = (o["hidden"] as? List<*>).orEmpty().filterIsInstance<String>()
            return ActivityRecord(entries, hidden, (o["cleared_at"] as? Number)?.toLong())
        }
    }
}

/**
 * The Activity timeline's rules, without Android (so JVM tests check them):
 * every source sorted once and walked lazily, a k-way merge taking the
 * newest across them until a page is full; paging from the last row shown
 * (its time and id); filters for a device, kinds and words (as [Search]:
 * every word in the subject, detail or device name, ignoring case and
 * accents). Hidden kinds are left out everywhere, and rows from before a
 * clear aren't listed.
 */
object ActivityFeed {
    val KINDS = listOf("files", "clipboard", "messages", "calls", "notifications", "connections", "other")
    const val PAGE = 50
    const val MAX_PAGE = 200
    /** The longest subject, in characters. */
    const val SUBJECT = 80

    fun kindOf(action: String): String = when {
        action.startsWith("file_") -> "files"
        action == "clip_in" || action == "clip_out" -> "clipboard"
        action.startsWith("msg_") -> "messages"
        action == "call" -> "calls"
        action == "notification" -> "notifications"
        action in setOf("link_up", "link_down", "key_changed", "wake") -> "connections"
        else -> "other"
    }

    /** Timeline order: newer first, then by id. Negative when [a] comes first. */
    fun compare(aTs: Long, aId: String, bTs: Long, bId: String): Int =
        if (aTs != bTs) bTs.compareTo(aTs) else aId.compareTo(bId)

    private val ORDER = Comparator<ActivityRow> { a, b -> compare(a.ts, a.id, b.ts, b.id) }

    /** A message's or clip's first line, at most [SUBJECT] characters. */
    fun firstLine(text: String): String {
        val line = text.lineSequence().map { it.trim() }.firstOrNull { it.isNotEmpty() } ?: ""
        val more = line.length > SUBJECT || text.trim().lines().size > 1
        return line.take(SUBJECT) + if (more) "…" else ""
    }

    fun transferRow(t: TransferRecord) = ActivityRow(
        "transfer:${t.id}", "files",
        when {
            // A link downloaded here, or on the other device ("Download on…").
            t.link != null && t.state == TransferState.Done -> if (t.incoming) "link_downloaded" else "link_downloaded_there"
            t.state == TransferState.Done -> if (t.incoming) "file_received" else "file_sent"
            t.state == TransferState.Failed -> if (t.incoming) "file_receive_failed" else "file_send_failed"
            else -> if (t.incoming) "file_receiving" else "file_sending"
        },
        t.node.ifEmpty { null }, t.ended ?: t.started, t.name,
        if (t.link != null && t.state == TransferState.Done) t.link else t.reason ?: "",
        t.total.takeIf { it > 0 }, t.via, t.id,
    )

    fun clipRow(c: ClipRecord) = ActivityRow(
        "clip:${c.id}", "clipboard", if (c.incoming) "clip_in" else "clip_out",
        c.node.ifEmpty { null }, c.at, firstLine(c.text), target = c.id.toString(),
    )

    fun messageRow(m: ChatEntry) = ActivityRow(
        "msg:${m.id}", "messages",
        when (m.status) {
            ChatStatus.Received -> "msg_in"
            ChatStatus.Failed -> "msg_failed"
            else -> "msg_out"
        },
        m.node.ifEmpty { null }, m.at, firstLine(m.text), target = m.bubble.toString(),
    )

    fun noteRow(n: NoteSource) = ActivityRow(
        "note:${n.id}", "other", "note", n.by?.ifEmpty { null }, n.updatedAt,
        firstLine(Search.noteTitle(n.title, n.body)), target = n.id,
    )

    fun linkRow(i: ReadLaterLinks.Item) = ActivityRow(
        "link:${i.id}", "other", "read_later", null, i.savedAt, firstLine(i.title.ifEmpty { i.url }), i.host, target = i.id,
    )

    fun timerRow(t: TimerSource) = ActivityRow(
        "timer:${t.id}", "other", "timer", t.by?.ifEmpty { null }, t.createdAt, t.label, t.durationMs.toString(), target = t.id,
    )

    fun logRow(e: ActivityEntry) = ActivityRow(
        "log:${e.id}", kindOf(e.action), e.action, e.node, e.ts, e.subject, e.detail, target = e.node ?: "",
    )

    /** [list] newest first, rows made by [make] only as they're taken; ones
     *  newer than [before] skipped without being made. Equal times in id order. */
    private fun <T> stream(list: List<T>, before: Long?, ts: (T) -> Long, make: (T) -> ActivityRow): Iterator<ActivityRow> {
        val order = list.sortedByDescending(ts)
        val from = if (before == null) 0 else order.indexOfFirst { ts(it) <= before }.let { if (it < 0) order.size else it }
        return object : Iterator<ActivityRow> {
            var i = from
            val held = ArrayDeque<ActivityRow>()
            override fun hasNext() = held.isNotEmpty() || i < order.size
            override fun next(): ActivityRow {
                if (held.isNotEmpty()) return held.removeFirst()
                val t0 = ts(order[i])
                var j = i
                while (j < order.size && ts(order[j]) == t0) j++
                if (j - i == 1) return make(order[i++])
                val run = (i until j).map { make(order[it]) }.sortedBy { it.id }
                i = j
                held.addAll(run)
                return held.removeFirst()
            }
        }
    }

    /** One page of the timeline from [src] for [q], leaving out [hidden]
     *  kinds and rows from before [clearedAt]. */
    fun page(src: ActivitySources, q: ActivityQuery, hidden: Collection<String> = emptyList(), clearedAt: Long? = null): ActivityPage {
        val limit = q.limit.coerceIn(1, MAX_PAGE)
        val asked = q.kinds.filter { it in KINDS }
        fun wanted(kind: String) = kind !in hidden && (asked.isEmpty() || kind in asked)
        val before = q.beforeTs
        val streams = ArrayList<Iterator<ActivityRow>>()
        if (wanted("files")) streams += stream(src.transfers, before, { it.ended ?: it.started }, ::transferRow)
        if (wanted("clipboard")) streams += stream(src.clips, before, { it.at }, ::clipRow)
        if (wanted("messages")) streams += stream(src.messages, before, { it.at }, ::messageRow)
        if (wanted("other")) {
            streams += stream(src.notes, before, { it.updatedAt }, ::noteRow)
            streams += stream(src.links, before, { it.savedAt }, ::linkRow)
            streams += stream(src.timers, before, { it.createdAt }, ::timerRow)
        }
        // The record holds several kinds: filtered row by row.
        if (hidden.size < KINDS.size) streams += stream(src.log, before, { it.ts }, ::logRow)

        val node = q.node?.ifEmpty { null }
        val words = Search.queryWords(q.text)
        fun keep(x: ActivityRow): Boolean {
            if (x.ts <= 0 || (clearedAt != null && x.ts < clearedAt)) return false
            if (before != null && compare(x.ts, x.id, before, q.beforeId ?: "") <= 0) return false
            if (!wanted(x.kind)) return false
            if (node != null && x.node != node) return false
            if (words.isNotEmpty()) {
                val name = x.node?.let { src.names[it] } ?: ""
                if (Search.score(words, x.subject, "${x.detail} $name") == null) return false
            }
            return true
        }
        fun pull(i: Int): Pair<ActivityRow, Int>? {
            val it = streams[i]
            while (it.hasNext()) {
                val x = it.next()
                if (keep(x)) return x to i
            }
            return null
        }
        val heap = PriorityQueue<Pair<ActivityRow, Int>>(maxOf(1, streams.size)) { a, b -> ORDER.compare(a.first, b.first) }
        for (i in streams.indices) pull(i)?.let(heap::add)
        val out = ArrayList<ActivityRow>(minOf(limit, 256))
        while (heap.isNotEmpty()) {
            val (row, from) = heap.poll() ?: break
            if (out.size == limit) return ActivityPage(out, true)
            out += row
            pull(from)?.let(heap::add)
        }
        return ActivityPage(out, false)
    }

    /** Rows grouped by day ([dayOf] gives a row's day key), in order. */
    fun <K> byDay(rows: List<ActivityRow>, dayOf: (Long) -> K): List<Pair<K, List<ActivityRow>>> {
        val out = ArrayList<Pair<K, MutableList<ActivityRow>>>()
        for (r in rows) {
            val d = dayOf(r.ts)
            if (out.isNotEmpty() && out.last().first == d) out.last().second += r else out += d to mutableListOf(r)
        }
        return out
    }
}
