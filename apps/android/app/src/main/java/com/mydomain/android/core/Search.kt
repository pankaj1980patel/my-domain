package com.mydomain.android.core

import java.text.Normalizer

/**
 * What the phone's search can find, in the order the groups show.
 *
 * Messages are the saved history ([ChatHistory]) and this session's chat
 * ([Core.messages]) together; with "Keep message history" off, only this
 * session's. Notes are the notepad's ([Notes]; deleted ones aren't there);
 * saved links are Read later's ([ReadLater]), found by title, site and link.
 */
enum class FoundKind {
    DEVICES,
    MESSAGES,
    NOTES,
    READ_LATER,
    CLIPBOARD,
    TRANSFERS,
    FOLDERS,
    AUTOMATIONS,
    AUTOMATION_APPS,
    SETTINGS,
    HELP,
}

/**
 * One thing that may be found: its fields to match ([title] counts most,
 * then [secondary], then [body]) and what the result shows. [title] empty:
 * the result shows a line of [body] around the match instead. [node]: the
 * device it belongs to; [ts]: when, in ms since 1970 (newer first among
 * equals); [tag]: a kind's own short note (a clip's way, a transfer's state).
 */
data class Findable(
    val kind: FoundKind,
    val id: String,
    val title: String,
    val secondary: String = "",
    val body: String = "",
    val node: String? = null,
    val ts: Long? = null,
    val tag: String? = null,
)

/** A message this session shows in a chat ([Core.messages]): [bubble] is
 *  its id there (what the chat marks), [msgId] the id it's saved under. */
data class SessionMessage(
    val bubble: Long,
    val msgId: String?,
    val node: String,
    val incoming: Boolean,
    val text: String,
    val ts: Long,
)

/** A note as searched (the core's note): [updatedAt] when it last changed,
 *  [by] the device that changed it (null: none to name, e.g. this phone),
 *  [conflict] whether it's a conflict copy. */
data class NoteSource(
    val id: String,
    val title: String,
    val body: String,
    val updatedAt: Long,
    val by: String? = null,
    val conflict: Boolean = false,
)

/** One result: what to show as its [title] and [detail] (a line of the
 *  text around the first match, "" when none), and its [score]. */
data class Found(val item: Findable, val title: String, val detail: String, val score: Double)

/** The hits of one kind, best first, and how many matched in all. */
data class FoundGroup(val kind: FoundKind, val total: Int, val hits: List<Found>)

/** A match in some text: [start] to [end] (exclusive), UTF-16 indices. */
data class MatchSpan(val start: Int, val end: Int)

/**
 * Search everything on the phone: the rules the desktop's search uses
 * (search.rs and ui/src/lib/search.ts), without Android, so JVM tests can
 * check them.
 *
 *  - every word of the query must be somewhere in an item, ignoring case
 *    and accents ("cafe" finds "Café", "CAFÉ" and a "café" written with a
 *    combining accent; typographic apostrophes match plain ones);
 *  - a word in the title counts most, then in the secondary field, then in
 *    the text; a word found at the start of a word counts a little more;
 *    newer items first among equals;
 *  - nothing is indexed or saved, and nothing leaves the phone.
 *
 * Folding takes each character to at most one, so a match in the folded
 * text points back into the original (for highlighting and snippets).
 */
object Search {
    /** The longest snippet, in characters. */
    const val SNIPPET = 140
    /** Hits a group shows before "Show all". */
    const val PREVIEW = 3
    /** The most hits one group keeps. */
    const val MAX_HITS = 200

    private const val TITLE = 3.0
    private const val SECONDARY = 2.0
    private const val BODY = 1.0
    private const val WORD_START = 0.5
    private const val DAY_MS = 24.0 * 3600.0 * 1000.0

    // --- folding -------------------------------------------------------------

    private fun isMark(cp: Int): Boolean = when (Character.getType(cp)) {
        Character.NON_SPACING_MARK.toInt(),
        Character.ENCLOSING_MARK.toInt(),
        Character.COMBINING_SPACING_MARK.toInt() -> true
        else -> false
    }

    /** One code point folded: -1 for a combining accent (dropped), else one
     *  code point (lower case, accent off, ‘ ’ as a plain apostrophe). */
    private fun foldCp(cp: Int): Int {
        if (isMark(cp)) return -1
        var base = cp
        if (cp >= 0x80) {
            val nfd = Normalizer.normalize(String(Character.toChars(cp)), Normalizer.Form.NFD)
            // Only when the rest is accents: Hangul and the like stay whole.
            if (nfd.codePointCount(0, nfd.length) > 1) {
                val first = nfd.codePointAt(0)
                var i = Character.charCount(first)
                var rest = true
                while (i < nfd.length) {
                    val c = nfd.codePointAt(i)
                    if (!isMark(c)) { rest = false; break }
                    i += Character.charCount(c)
                }
                if (rest) base = first
            }
        }
        val lower = String(Character.toChars(base)).lowercase().codePointAt(0)
        return if (lower == 0x2018 || lower == 0x2019) '\''.code else lower
    }

    private fun isAscii(s: String): Boolean {
        for (ch in s) if (ch.code >= 0x80) return false
        return true
    }

    /** [s] folded for matching: lower case, no accents. */
    fun fold(s: String): String {
        if (isAscii(s)) return s.lowercase()
        val out = StringBuilder(s.length)
        var i = 0
        while (i < s.length) {
            val cp = s.codePointAt(i)
            val f = foldCp(cp)
            if (f >= 0) out.appendCodePoint(f)
            i += Character.charCount(cp)
        }
        return out.toString()
    }

    /** The words of a query, folded. */
    fun queryWords(query: String): List<String> =
        fold(query).split(WHITESPACE).filter { it.isNotEmpty() }

    private val WHITESPACE = Regex("""\s+""")

    /** [text] folded, with each folded UTF-16 unit's index in the original. */
    private fun foldMapped(text: String): Pair<String, IntArray> {
        val out = StringBuilder(text.length)
        val at = IntArray(text.length * 2)
        var n = 0
        var i = 0
        while (i < text.length) {
            val cp = text.codePointAt(i)
            val f = foldCp(cp)
            if (f >= 0) {
                repeat(Character.charCount(f)) { at[n++] = i }
                out.appendCodePoint(f)
            }
            i += Character.charCount(cp)
        }
        return out.toString() to at.copyOf(n)
    }

    /** Where [words] are in [text]: spans of the original, sorted and
     *  merged (accents after a match are in it). */
    fun matchRanges(text: String, words: List<String>): List<MatchSpan> {
        if (text.isEmpty() || words.isEmpty()) return emptyList()
        val (folded, at) = foldMapped(text)
        val spans = ArrayList<MatchSpan>()
        for (w in words) {
            if (w.isEmpty()) continue
            var from = 0
            while (true) {
                val i = folded.indexOf(w, from)
                if (i < 0) break
                val end = i + w.length
                spans += MatchSpan(at[i], if (end < at.size) at[end] else text.length)
                from = i + maxOf(1, w.length)
            }
        }
        spans.sortBy { it.start }
        val merged = ArrayList<MatchSpan>()
        for (s in spans) {
            val last = merged.lastOrNull()
            if (last != null && s.start <= last.end) merged[merged.size - 1] = MatchSpan(last.start, maxOf(last.end, s.end))
            else merged += s
        }
        return merged
    }

    /** [text] on one line, at most [max] characters, around the first match
     *  (from the start when none); an ellipsis where it was cut. */
    fun snippet(text: String, words: List<String>, max: Int = SNIPPET): String {
        val flat = text.split(WHITESPACE).filter { it.isNotEmpty() }.joinToString(" ")
        if (flat.length <= max) return flat
        val first = matchRanges(flat, words).firstOrNull()?.start ?: 0
        var begin = maxOf(0, first - max / 4)
        var end = minOf(flat.length, begin + max)
        begin = maxOf(0, end - max)
        // Never half a surrogate pair.
        if (begin > 0 && Character.isLowSurrogate(flat[begin])) begin++
        if (end < flat.length && Character.isLowSurrogate(flat[end])) end--
        val sb = StringBuilder()
        if (begin > 0) sb.append('…')
        sb.append(flat, begin, end)
        if (end < flat.length) sb.append('…')
        return sb.toString()
    }

    // --- scoring -------------------------------------------------------------

    private fun isWordChar(cp: Int) = Character.isLetterOrDigit(cp)

    /** How much [word] counts in [hay] (folded) worth [weight]: 0 when absent. */
    private fun wordIn(hay: String, word: String, weight: Double): Double {
        var best = 0.0
        var from = 0
        while (true) {
            val i = hay.indexOf(word, from)
            if (i < 0) return best
            if (i == 0 || !isWordChar(Character.codePointBefore(hay, i))) return weight + WORD_START
            best = weight
            from = i + 1
        }
    }

    /** How well the fields match [words]: null unless every word is in one;
     *  each word counts for the best field it's in. No words: 0. */
    fun score(words: List<String>, title: String, secondary: String = "", body: String = ""): Double? {
        if (words.isEmpty()) return 0.0
        val t = fold(title)
        val s2 = fold(secondary)
        var b: String? = null
        var total = 0.0
        for (w in words) {
            var s = wordIn(t, w, TITLE)
            if (s < TITLE) s = maxOf(s, wordIn(s2, w, SECONDARY))
            if (s == 0.0) {
                val folded = b ?: fold(body).also { b = it }
                s = wordIn(folded, w, BODY)
            }
            if (s == 0.0) return null
            total += s
        }
        return total
    }

    /** Up to 1 for something new, towards 0 with age (half after a week). */
    fun recency(ts: Long?, now: Long): Double {
        if (ts == null || ts <= 0) return 0.0
        val days = maxOf(0.0, (now - ts) / DAY_MS)
        return 1.0 / (1.0 + days / 7.0)
    }

    // --- sources -------------------------------------------------------------

    /**
     * Messages as searched: the [saved] history merged with this session's
     * [live] chat by message id (the live copy wins: it has the bubble the
     * chat shows). A result's id is its bubble's id in the chat
     * ([ChatEntry.bubble] for a saved one), tagged [sent] or [received].
     */
    fun messageFindables(
        saved: List<ChatEntry>,
        live: List<SessionMessage>,
        sent: String,
        received: String,
    ): List<Findable> {
        val liveIds = live.mapNotNullTo(HashSet()) { it.msgId }
        val older = saved.filter { it.id !in liveIds }.map { e ->
            Findable(
                FoundKind.MESSAGES, e.bubble.toString(), "", body = e.text, node = e.node.ifEmpty { null }, ts = e.at,
                tag = if (e.incoming) received else sent,
            )
        }
        val now = live.map { m ->
            Findable(
                FoundKind.MESSAGES, m.bubble.toString(), "", body = m.text, node = m.node.ifEmpty { null }, ts = m.ts,
                tag = if (m.incoming) received else sent,
            )
        }
        return older + now
    }

    /** A note's name in a list: its title, else its text's first line. */
    fun noteTitle(title: String, body: String): String =
        title.trim().ifEmpty { body.lineSequence().map { it.trim() }.firstOrNull { it.isNotEmpty() } ?: "" }

    /** Notes as searched: by title (or first line) and text, dated by their
     *  last change; conflict copies tagged [conflictTag]. */
    fun noteFindables(notes: List<NoteSource>, conflictTag: String): List<Findable> = notes.map { n ->
        Findable(
            FoundKind.NOTES, n.id, noteTitle(n.title, n.body), body = n.body,
            node = n.by?.ifEmpty { null }, ts = n.updatedAt.takeIf { it > 0 },
            tag = if (n.conflict) conflictTag else null,
        )
    }

    /** Saved links as searched: by title, host and link, dated by when they
     *  were saved; tagged [readTag] or [unreadTag]. */
    fun readLaterFindables(items: List<ReadLaterLinks.Item>, readTag: String, unreadTag: String): List<Findable> =
        items.map { i ->
            Findable(
                FoundKind.READ_LATER, i.id, i.title.ifEmpty { i.url }, i.host, i.url,
                ts = i.savedAt.takeIf { it > 0 }, tag = if (i.read) readTag else unreadTag,
            )
        }

    // --- ranking -------------------------------------------------------------

    /** The best [limit] of [items] for [words], as one group (null when
     *  nothing matched). */
    fun rank(kind: FoundKind, words: List<String>, items: List<Findable>, limit: Int, now: Long): FoundGroup? {
        val scored = items.mapNotNull { f -> score(words, f.title, f.secondary, f.body)?.let { f to it + recency(f.ts, now) } }
        if (scored.isEmpty()) return null
        val sorted = scored.sortedWith(
            compareByDescending<Pair<Findable, Double>> { it.second }
                .thenByDescending { it.first.ts ?: 0L }
                .thenBy { it.first.title },
        )
        val hits = sorted.take(limit.coerceAtLeast(1)).map { (f, s) ->
            // The text around the match, unless the title says it all.
            if (f.title.isEmpty()) Found(f, snippet(f.body, words), "", s)
            else Found(f, f.title, if (f.body.isEmpty()) "" else snippet(f.body, words), s)
        }
        return FoundGroup(kind, scored.size, hits)
    }

    /**
     * Search [items] for [query]: the groups that matched, in [FoundKind]
     * order, each with its best [limit] hits. [kinds]: only these (all when
     * empty). An empty query finds nothing.
     */
    fun search(
        query: String,
        items: List<Findable>,
        kinds: Set<FoundKind> = emptySet(),
        limit: Int = MAX_HITS,
        now: Long = System.currentTimeMillis(),
    ): List<FoundGroup> {
        val words = queryWords(query)
        if (words.isEmpty()) return emptyList()
        val byKind = items.filter { kinds.isEmpty() || it.kind in kinds }.groupBy { it.kind }
        return FoundKind.entries.mapNotNull { k -> byKind[k]?.let { rank(k, words, it, limit, now) } }
    }

    /** The hits a group shows: the first [PREVIEW], or all when [expanded]. */
    fun shown(group: FoundGroup, expanded: Boolean): List<Found> =
        if (expanded) group.hits else group.hits.take(PREVIEW)

    /** Whether a group has more than it shows folded ("Show all N"). */
    fun hasMore(group: FoundGroup): Boolean = group.total > PREVIEW
}
