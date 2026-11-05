package com.mydomain.android.core

/**
 * What the Notes home-screen widget shows (the pure part of `NotesWidget`):
 * pinned notes first, then the most recently changed, each as a title and
 * one line, with a bell when it has a reminder still to come (or due and
 * not Done), and as many rows as the widget's height has room for.
 */
object NotesGlance {
    /** The most rows the widget's layout has. */
    const val MAX_ROWS = 5

    /** A row's longest title and line, in characters (the widget ellipsizes
     *  too; this keeps what's stored small). */
    const val MAX_TITLE = 80
    const val MAX_LINE = 120

    /** A note, as far as the widget cares. */
    data class Note(
        val id: String,
        val title: String,
        val body: String,
        val pinned: Boolean,
        val updatedAt: Long,
        val remindAt: Long?,
        val remindDone: Boolean,
    )

    /** One row: [title] null means "Untitled" (no title and no text). */
    data class Row(val id: String, val title: String?, val line: String, val bell: Boolean)

    /** The rows to keep for the widget: pinned first, then the most
     *  recently changed (ties by id), at most [max]. */
    fun pick(notes: Collection<Note>, max: Int = MAX_ROWS): List<Row> =
        notes.sortedWith(
            compareByDescending<Note> { it.pinned }.thenByDescending { it.updatedAt }.thenBy { it.id },
        ).take(max.coerceAtLeast(0)).map(::row)

    /** A note as a row. A note without a title is shown by its first line,
     *  with the next one under it. Checklist boxes ("[ ] ", "[x] ") are
     *  left out of the text. */
    fun row(n: Note): Row {
        val lines = textLines(n.body)
        val title = tidy(n.title)
        return if (title.isNotEmpty()) {
            Row(n.id, clip(title, MAX_TITLE), clip(lines.firstOrNull() ?: "", MAX_LINE), bell(n))
        } else {
            Row(n.id, lines.firstOrNull()?.let { clip(it, MAX_TITLE) }, clip(lines.getOrNull(1) ?: "", MAX_LINE), bell(n))
        }
    }

    /** A reminder still to come, or due and not Done (as the app's list). */
    fun bell(n: Note): Boolean = n.remindAt != null && !n.remindDone

    private val BOX = Regex("^\\[( |x|X)] ")
    private val SPACE = Regex("\\s+")

    /** The body's lines that aren't blank, tidied, boxes left out. */
    fun textLines(body: String): List<String> =
        body.lineSequence().map { tidy(it.trim().replace(BOX, "")) }.filter { it.isNotEmpty() }.take(2).toList()

    private fun tidy(s: String): String = s.replace(SPACE, " ").trim()

    /** [s] cut to [max] characters, with "…" when it was longer. */
    fun clip(s: String, max: Int): String {
        if (s.length <= max) return s
        if (max <= 1) return "…".take(max)
        // Don't split a surrogate pair (an emoji) in two.
        var end = max - 1
        if (end > 0 && Character.isHighSurrogate(s[end - 1])) end--
        return s.substring(0, end).trimEnd() + "…"
    }

    /** The widget's header and padding, and one row with its gap, in dp
     *  (res/layout/notes_widget.xml). */
    const val HEADER_DP = 54
    const val ROW_DP = 52

    /** How many rows fit a widget [heightDp] tall (unknown, 0: three). */
    fun rowsFor(heightDp: Int): Int {
        if (heightDp <= 0) return 3
        return ((heightDp - HEADER_DP) / ROW_DP).coerceIn(1, MAX_ROWS)
    }
}
