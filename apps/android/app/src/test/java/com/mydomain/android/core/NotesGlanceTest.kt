package com.mydomain.android.core

import com.mydomain.android.core.NotesGlance.Note
import com.mydomain.android.core.NotesGlance.Row
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class NotesGlanceTest {
    private fun note(
        id: String,
        title: String = "",
        body: String = "",
        pinned: Boolean = false,
        updatedAt: Long = 0,
        remindAt: Long? = null,
        remindDone: Boolean = false,
    ) = Note(id, title, body, pinned, updatedAt, remindAt, remindDone)

    @Test
    fun pinnedFirstThenRecent() {
        val notes = listOf(
            note("a", "A", updatedAt = 10),
            note("b", "B", updatedAt = 30),
            note("p", "P", pinned = true, updatedAt = 1),
            note("c", "C", updatedAt = 20),
            note("q", "Q", pinned = true, updatedAt = 5),
        )
        assertEquals(listOf("q", "p", "b", "c", "a"), NotesGlance.pick(notes).map { it.id })
    }

    @Test
    fun tiesByIdAndLimit() {
        val notes = (1..8).map { note("n$it", "N$it", updatedAt = 7) }
        val rows = NotesGlance.pick(notes, 3)
        assertEquals(listOf("n1", "n2", "n3"), rows.map { it.id })
        assertEquals(NotesGlance.MAX_ROWS, NotesGlance.pick(notes).size)
        assertTrue(NotesGlance.pick(notes, 0).isEmpty())
        assertTrue(NotesGlance.pick(emptyList()).isEmpty())
    }

    @Test
    fun titleAndFirstLine() {
        val r = NotesGlance.row(note("x", "  Shopping  ", "\n\n  [ ] milk\n[x] eggs"))
        assertEquals(Row("x", "Shopping", "milk", false), r)
    }

    @Test
    fun untitledUsesFirstLines() {
        assertEquals(Row("x", "Call Sam", "about Friday", false), NotesGlance.row(note("x", " ", "Call Sam\n\nabout Friday\nmore")))
        assertEquals(Row("x", "only line", "", false), NotesGlance.row(note("x", body = "only line")))
        val empty = NotesGlance.row(note("x"))
        assertNull(empty.title)
        assertEquals("", empty.line)
    }

    @Test
    fun whitespaceCollapsed() {
        assertEquals("a b c", NotesGlance.row(note("x", "a\t b   c")).title)
        assertEquals("one two", NotesGlance.row(note("x", "T", "one    two")).line)
    }

    @Test
    fun bellOnlyForPendingReminders() {
        assertTrue(NotesGlance.row(note("x", "T", remindAt = 5)).bell)
        assertFalse(NotesGlance.row(note("x", "T", remindAt = 5, remindDone = true)).bell)
        assertFalse(NotesGlance.row(note("x", "T")).bell)
    }

    @Test
    fun clipping() {
        assertEquals("abc", NotesGlance.clip("abc", 3))
        assertEquals("ab…", NotesGlance.clip("abcd", 3))
        assertEquals("ab…", NotesGlance.clip("ab  cd", 4))
        // An emoji (two chars) isn't cut in half.
        assertEquals("a…", NotesGlance.clip("a😀bc", 3))
        val long = "x".repeat(500)
        assertEquals(NotesGlance.MAX_TITLE, NotesGlance.row(note("x", long)).title!!.length)
        assertEquals(NotesGlance.MAX_LINE, NotesGlance.row(note("x", "T", long)).line.length)
    }

    @Test
    fun rowsForHeight() {
        assertEquals(3, NotesGlance.rowsFor(0))
        assertEquals(1, NotesGlance.rowsFor(40))
        assertEquals(1, NotesGlance.rowsFor(110))
        assertEquals(2, NotesGlance.rowsFor(160))
        assertEquals(3, NotesGlance.rowsFor(210))
        assertEquals(5, NotesGlance.rowsFor(1000))
    }
}
