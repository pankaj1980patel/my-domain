package com.mydomain.android.core

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class TransferBookTest {

    private fun finish(b: TransferBook, i: Int) {
        val id = "t$i"
        b.progress(id, "p", "a.txt", 1, 10, false, false, i.toLong())
        assertTrue(b.sent(id, "p", "a.txt", i.toLong()))
    }

    @Test
    fun keepsTheNewestFinishedUpToTheCap() {
        val b = TransferBook()
        b.progress("live", "p", "big.mov", 5, 100, true, false, 0)
        for (i in 1..TransferBook.MAX_FINISHED + 20) finish(b, i)
        val list = b.list
        // The one under way is never trimmed and comes first.
        assertEquals("live", list.first().id)
        assertEquals(TransferBook.MAX_FINISHED + 1, list.size)
        assertEquals("t${TransferBook.MAX_FINISHED + 20}", list[1].id)
        assertEquals("t21", list.last().id)
        assertEquals(TransferBook.MAX_FINISHED, b.saved().size)
    }

    @Test
    fun eventsUpdateOneEntryById() {
        val b = TransferBook()
        b.sending("x", "p", "photo.jpg", "content://pick/1", 100, 1000)
        b.progress("x", "p", "photo.jpg", 40, 100, false, true, 2000)
        b.progress("x", "p", "photo.jpg", 70, 100, false, true, 3000)
        assertEquals(1, b.list.size)
        val e = b.get("x")!!
        assertEquals("content://pick/1", e.source)
        assertEquals(40L, e.resumedFrom)
        assertEquals(70L, e.done)
        assertEquals(1000L, e.started)
        // 30 bytes in a second.
        assertEquals(30.0, e.rate, 0.001)
        assertEquals(1L, e.secondsLeft())
        assertTrue(b.sent("x", "p", "photo.jpg", 4000))
        // Done stays done: a late failure or progress changes nothing.
        assertFalse(b.failed("x", "p", "photo.jpg", false, "cancelled", 5000))
        b.progress("x", "p", "photo.jpg", 1, 100, false, false, 6000)
        val d = b.get("x")!!
        assertEquals(TransferState.Done, d.state)
        assertEquals(100L, d.done)
        assertEquals(4000L, d.ended)

        assertTrue(b.received("y", "q", "in.pdf", 9, 7000))
        assertTrue(b.saved("y", "content://downloads/9", "application/pdf"))
        assertFalse(b.saved("nope", "content://x", null))
        assertTrue(b.failed("z", "q", "c.bin", true, "cancelled", 8000))
        assertEquals(listOf("z", "y", "x"), b.list.map { it.id })
        assertEquals("content://downloads/9", b.get("y")!!.uri)
        assertEquals("cancelled", b.get("z")!!.reason)
    }

    @Test
    fun clearFinishedKeepsThoseUnderWay() {
        val b = TransferBook()
        finish(b, 1)
        b.progress("live", "p", "b", 1, 2, true, false, 2)
        assertTrue(b.clearFinished())
        assertFalse(b.clearFinished())
        assertEquals(listOf("live"), b.list.map { it.id })
    }

    @Test
    fun savedAndReadBack() {
        val b = TransferBook()
        b.progress("a", "p", "a \"quoted\".txt", 3, 10, false, true, 1)
        b.sending("a", "p", "a \"quoted\".txt", "content://pick/a", 10, 1)
        b.sent("a", "p", "a \"quoted\".txt", 2)
        b.received("b", "q", "b.txt", 5, 3)
        b.saved("b", "content://dl/b", "text/plain")
        b.progress("live", "p", "c", 1, 2, true, false, 4)
        val text = TransferBook.encode(b.list)
        val back = TransferBook.decode(text)
        // Only finished ones are saved.
        assertEquals(listOf("b", "a"), back.map { it.id })
        val a = back[1]
        assertEquals("a \"quoted\".txt", a.name)
        assertEquals("content://pick/a", a.source)
        assertEquals(3L, a.resumedFrom)
        assertEquals(10L, a.done)
        assertEquals(TransferState.Done, a.state)
        assertFalse(a.incoming)
        assertEquals("content://dl/b", back[0].uri)
        assertEquals("text/plain", back[0].mime)
        assertNull(back[0].resumedFrom)
        // A book from the file, with a newer one arriving too.
        val again = TransferBook(back)
        again.addSaved(back)
        assertEquals(2, again.list.size)
        assertEquals(emptyList<TransferRecord>(), TransferBook.decode("not json"))
    }
}
