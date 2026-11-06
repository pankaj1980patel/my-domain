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

    @Test
    fun transfersOverASideRouteSaySo() {
        val b = TransferBook()
        b.progress("going", "phone", "big.mov", 5, 100, false, false, 0)
        b.progress("other", "laptop", "x.zip", 5, 100, false, false, 0)
        finish(b, 1)
        b.wifiDirect("phone", true, TransferBook.VIA_WIFI_DIRECT)
        // One that starts while it's up says so too.
        b.progress("next", "phone", "b.mov", 1, 100, true, false, 1)
        assertEquals("wifi_direct", b.get("going")?.via)
        assertEquals("wifi_direct", b.get("next")?.via)
        assertNull("another device's transfer", b.get("other")?.via)
        assertNull("one that ended before", b.get("t1")?.via)
        // Down: new ones don't; the ones that used it still do, saved too.
        b.wifiDirect("phone", false)
        b.progress("later", "phone", "c.mov", 1, 100, true, false, 2)
        assertNull(b.get("later")?.via)
        assertTrue(b.sent("going", "phone", "big.mov", 3))
        val back = TransferBook.decode(TransferBook.encode(b.saved()))
        assertEquals("wifi_direct", back.first { it.id == "going" }.via)
        // A Mac over the phone's hotspot.
        b.wifiDirect("mac", true, TransferBook.VIA_HOTSPOT)
        b.progress("hs", "mac", "d.mov", 1, 100, false, false, 4)
        assertEquals("hotspot", b.get("hs")?.via)
    }

    @Test
    fun speedIsSmoothedAndGivesTimeLeft() {
        val b = TransferBook()
        b.progress("x", "p", "a", 0, 10_000, false, false, 1000)
        b.progress("x", "p", "a", 1000, 10_000, false, false, 2000) // 1000 B/s
        assertEquals(1000.0, b.get("x")!!.rate, 1e-9)
        b.progress("x", "p", "a", 4000, 10_000, false, false, 3000) // 3000 B/s weighed 0.3
        val e = b.get("x")!!
        assertEquals(1600.0, e.rate, 1e-9)
        assertEquals(4L, e.secondsLeft()) // 6000 / 1600 = 3.75
        assertEquals(1600.0, e.rateAt(3000 + TransferRecord.STALE_MS), 1e-9)
        assertEquals(0.0, e.rateAt(3001 + TransferRecord.STALE_MS), 1e-9)
        // Paused: no speed, no time left.
        assertTrue(b.paused("x", true))
        assertFalse(b.paused("x", true))
        assertNull(b.get("x")!!.secondsLeft())
        assertEquals(0.0, b.get("x")!!.rateAt(3000), 1e-9)
        // Started over from less: the old speed goes.
        b.paused("x", false)
        b.progress("x", "p", "a", 100, 10_000, false, false, 9000)
        assertEquals(0.0, b.get("x")!!.rate, 1e-9)
    }

    private fun batch(b: TransferBook, state: String, done: Long, filesDone: Int = 1, now: Long = 1): Boolean =
        b.batch("b1", "p", "Trip", true, "folder", 3, filesDone, 0, done, 300, state, null, now)

    @Test
    fun aBatchIsOneRowWithItsFiles() {
        val b = TransferBook()
        assertFalse(batch(b, "active", 0))
        b.progress("f1", "p", "a.jpg", 50, 100, true, false, 2, batch = "b1")
        b.progress("other", "q", "x.zip", 5, 10, true, false, 3)
        assertEquals(listOf("other", "b1"), b.rows.map { it.id })
        assertEquals(listOf("f1"), b.filesOf("b1").map { it.id })
        val row = b.get("b1")!!
        assertTrue(row.isBatch)
        assertEquals(3, row.count)
        assertFalse(batch(b, "paused", 100, now = 4))
        assertTrue(b.get("b1")!!.paused)
        assertTrue(batch(b, "done", 300, 3, now = 5))
        assertEquals(TransferState.Done, b.get("b1")!!.state)
        assertEquals(5L, b.get("b1")!!.ended)
        // Sent again after failing: under way once more.
        assertTrue(b.batch("b1", "p", "Trip", true, "folder", 3, 2, 1, 200, 300, "failed", "the device wasn't reachable", 6))
        assertEquals("the device wasn't reachable", b.get("b1")!!.reason)
        assertTrue(batch(b, "active", 200, now = 7))
        assertEquals(TransferState.Active, b.get("b1")!!.state)
        assertNull(b.get("b1")!!.ended)
    }

    @Test
    fun aBigBatchDoesNotPushOutOtherTransfers() {
        val b = TransferBook()
        finish(b, 1)
        batch(b, "active", 0)
        for (i in 0 until 500) {
            b.progress("f$i", "p", "x", 1, 1, true, false, 2, batch = "b1")
            b.received("f$i", "p", "x", 1, 3L + i)
        }
        batch(b, "done", 300, 3, now = 600)
        assertTrue(b.get("t1") != null)
        assertEquals(TransferBook.MAX_BATCH_FILES, b.filesOf("b1").size)
    }

    @Test
    fun batchFieldsAreSavedAndRead() {
        val b = TransferBook()
        batch(b, "done", 300, 3)
        b.progress("f1", "p", "a.jpg", 1, 1, true, false, 2, batch = "b1")
        b.received("f1", "p", "a.jpg", 1, 3)
        val back = TransferBook.decode(TransferBook.encode(b.saved()))
        val row = back.first { it.id == "b1" }
        assertEquals("folder", row.kind)
        assertEquals(3, row.count)
        assertEquals("b1", back.first { it.id == "f1" }.batch)
    }
}
