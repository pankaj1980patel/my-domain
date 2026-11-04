package com.mydomain.android.core

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class ClipBookTest {

    private fun clip(id: Long, incoming: Boolean, node: String, text: String, truncated: Boolean = false) =
        ClipRecord(id, text, truncated, node, incoming, id * 1000)

    @Test
    fun keepsTheNewestUpToTheCap() {
        var list = emptyList<ClipRecord>()
        for (id in 1L..7L) list = ClipBook.record(list, clip(id, true, "p", "hi"), 5)
        assertEquals(listOf(7L, 6L, 5L, 4L, 3L), list.map { it.id })
    }

    @Test
    fun theRealCapIs200() {
        val book = ClipBook()
        for (i in 1..250) book.add("p", false, "x$i", i.toLong())
        assertEquals(200, book.list.size)
        assertEquals("x250", book.list.first().text)
        assertEquals("x51", book.list.last().text)
        assertEquals(200, book.list.map { it.id }.toSet().size)
    }

    @Test
    fun blankTextOrNoDeviceIsNotKept() {
        val book = ClipBook()
        assertNull(book.add("p", true, "   \n", 1))
        assertNull(book.add("", true, "hi", 1))
        assertTrue(book.list.isEmpty())
    }

    @Test
    fun longTextIsCutAndMarked() {
        assertEquals("short" to false, ClipBook.capText("short", 10))
        assertEquals("exactly10!" to false, ClipBook.capText("exactly10!", 10))
        assertEquals("0123456789" to true, ClipBook.capText("0123456789abc", 10))
        val (kept, cut) = ClipBook.capText("a".repeat(ClipBook.MAX_TEXT + 1), ClipBook.MAX_TEXT)
        assertTrue(cut)
        assertEquals(10 * 1024, kept.length)

        val e = ClipBook().add("p", true, "b".repeat(20_000), 1)!!
        assertTrue(e.truncated)
        assertEquals(10 * 1024, e.text.toByteArray(Charsets.UTF_8).size)
    }

    @Test
    fun cuttingKeepsWholeCharacters() {
        // "é" is two bytes: a cut through it backs off to before it.
        assertEquals("a" to true, ClipBook.capText("aé", 2))
        // "€" is three bytes.
        assertEquals("a" to true, ClipBook.capText("a€", 3))
        assertEquals("a€" to false, ClipBook.capText("a€", 4))
        // "🙂" is a surrogate pair, four bytes: never split.
        assertEquals("🙂" to true, ClipBook.capText("🙂🙂", 6))
        assertEquals("a" to true, ClipBook.capText("a🙂", 4))
        assertEquals("a🙂" to false, ClipBook.capText("a🙂", 5))
        // What's kept always fits in the byte budget.
        val mixed = "aé€🙂".repeat(1000)
        val (kept, cut) = ClipBook.capText(mixed, 1001)
        assertTrue(cut)
        assertTrue(kept.toByteArray(Charsets.UTF_8).size <= 1001)
        assertFalse(kept.last().isHighSurrogate())
    }

    @Test
    fun searchMatchesEveryWordIgnoringCase() {
        val text = "Meet at the Blue Door, 8pm"
        assertTrue(ClipBook.matches(text, ""))
        assertTrue(ClipBook.matches(text, "   "))
        assertTrue(ClipBook.matches(text, "blue"))
        assertTrue(ClipBook.matches(text, "DOOR  blue"))
        assertFalse(ClipBook.matches(text, "blue window"))
    }

    @Test
    fun pagesKeepToTheDeviceEitherWay() {
        val book = ClipBook(
            listOf(
                clip(5, false, "p", "hello sent"),
                clip(4, true, "q", "hello other"),
                clip(3, true, "p", "hello received"),
                clip(2, true, "p", "bye"),
                clip(1, false, "q", "to q"),
            ),
        )
        val all = book.page("p", "", 30)
        assertEquals(listOf(5L, 3L, 2L), all.entries.map { it.id })
        assertEquals(3, all.total)
        assertEquals(3, all.kept)

        val hello = book.page("p", "Hello", 30)
        assertEquals(listOf(5L, 3L), hello.entries.map { it.id })
        assertEquals(2, hello.total)
        assertEquals(3, hello.kept)

        val first = book.page("p", "", 1)
        assertEquals(listOf(5L), first.entries.map { it.id })
        assertEquals(3, first.total)

        assertEquals(0, book.page("x", "", 30).kept)
    }

    @Test
    fun clearForgetsOneDevice() {
        val book = ClipBook(listOf(clip(2, true, "p", "a"), clip(1, false, "q", "b")))
        assertTrue(book.clear("p"))
        assertFalse(book.clear("p"))
        assertEquals(listOf(1L), book.list.map { it.id })
        book.clearAll()
        assertTrue(book.list.isEmpty())
    }

    @Test
    fun newIdsFollowTheKeptOnes() {
        val book = ClipBook(listOf(clip(9, true, "p", "old")))
        assertEquals(10L, book.add("p", false, "new", 1)!!.id)
    }

    @Test
    fun olderClipsGoAfterNewerOnesWithoutSharingIds() {
        val book = ClipBook()
        book.add("p", true, "new one", 100) // id 1
        book.add("p", false, "new two", 200) // id 2
        book.addOlder(listOf(clip(2, true, "q", "old two"), clip(1, true, "q", "old one")))
        assertEquals(listOf("new two", "new one", "old two", "old one"), book.list.map { it.text })
        assertEquals(4, book.list.map { it.id }.toSet().size)
        val next = book.add("p", true, "newest", 300)!!
        assertTrue(book.list.drop(1).all { it.id != next.id })
    }

    @Test
    fun theFileRoundTrips() {
        val list = listOf(
            clip(2, false, "p", "sent \"quoted\"\nline 2 é 🙂"),
            clip(1, true, "q", "got", truncated = true),
        )
        val json = ClipBook.encode(list)
        assertTrue(json.contains("\"direction\":\"out\""))
        assertTrue(json.contains("\"direction\":\"in\""))
        assertEquals(list, ClipBook.decode(json))
    }

    @Test
    fun badFilesReadAsNothingOrSkipBadEntries() {
        assertTrue(ClipBook.decode("").isEmpty())
        assertTrue(ClipBook.decode("not json").isEmpty())
        assertTrue(ClipBook.decode("{\"a\":1}").isEmpty())
        val json = """[
            {"id":3,"text":"ok","node":"p","direction":"in","at":5},
            {"id":2,"text":"no direction","node":"p","at":5},
            {"id":1,"text":"sideways","node":"p","direction":"up","at":5},
            {"text":"no id","node":"p","direction":"out","at":5},
            {"id":4,"text":"   ","node":"p","direction":"out","at":5},
            7
        ]"""
        val back = ClipBook.decode(json)
        assertEquals(listOf(3L), back.map { it.id })
        assertFalse(back[0].truncated)
        assertEquals(5L, back[0].at)
    }

    @Test
    fun aFileTooLongIsCutOnReading() {
        val big = (1L..250L).map { clip(it, true, "p", "x") }
        assertEquals(200, ClipBook.decode(ClipBook.encode(big)).size)
        val huge = ClipBook.decode(ClipBook.encode(listOf(clip(1, true, "p", "z".repeat(20_000)))))
        assertTrue(huge[0].truncated)
        assertEquals(ClipBook.MAX_TEXT, huge[0].text.length)
    }
}
