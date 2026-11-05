package com.mydomain.android.core

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class SearchTest {
    private val now = 1_800_000_000_000L
    private val day = 24L * 3600 * 1000

    private fun item(
        kind: FoundKind,
        id: String,
        title: String = "",
        secondary: String = "",
        body: String = "",
        ts: Long? = null,
    ) = Findable(kind, id, title, secondary, body, ts = ts)

    // --- folding ---

    @Test
    fun foldsCaseAndAccents() {
        assertEquals("cafe", Search.fold("Café"))
        assertEquals("cafe", Search.fold("CAFÉ"))
        // "e" + combining acute accent (NFD).
        assertEquals("cafe", Search.fold("café"))
        assertEquals("naive resume", Search.fold("Naïve Résumé"))
        assertEquals("i", Search.fold("İ"))
        assertEquals("don't", Search.fold("Don’t"))
        assertEquals("plain ascii", Search.fold("Plain ASCII"))
    }

    @Test
    fun hangulStaysWhole() {
        assertEquals("한국", Search.fold("한국"))
    }

    @Test
    fun queryWordsAreFoldedAndSplit() {
        assertEquals(listOf("cafe", "creme"), Search.queryWords("  Café\tCRÈME \n"))
        assertTrue(Search.queryWords("   ").isEmpty())
    }

    // --- matching ---

    @Test
    fun everyWordMustMatch() {
        val w = Search.queryWords("cafe paris")
        assertNotNull(Search.score(w, "Café", body = "in Paris"))
        assertNull(Search.score(w, "Café", body = "in Lyon"))
    }

    @Test
    fun accentInsensitiveBothWays() {
        assertNotNull(Search.score(Search.queryWords("café"), "CAFE menu"))
        assertNotNull(Search.score(Search.queryWords("cafe"), "café menu"))
        assertNotNull(Search.score(Search.queryWords("don't"), "Don’t forget"))
    }

    @Test
    fun titleBeatsSecondaryBeatsBody() {
        val w = Search.queryWords("report")
        val title = Search.score(w, "report", "x", "x")!!
        val secondary = Search.score(w, "x", "report", "x")!!
        val body = Search.score(w, "x", "x", "report")!!
        assertTrue(title > secondary)
        assertTrue(secondary > body)
    }

    @Test
    fun wordStartCountsMore() {
        val w = Search.queryWords("port")
        val start = Search.score(w, "port forwarding")!!
        val inside = Search.score(w, "report")!!
        assertEquals(3.5, start, 1e-9)
        assertEquals(3.0, inside, 1e-9)
        // After punctuation is a word start too.
        assertEquals(3.5, Search.score(w, "re-port")!!, 1e-9)
    }

    @Test
    fun emptyQueryFindsNothing() {
        val items = listOf(item(FoundKind.DEVICES, "a", "Pixel"))
        assertTrue(Search.search("  ", items).isEmpty())
    }

    // --- ranking ---

    @Test
    fun newerWinsTies() {
        val items = listOf(
            item(FoundKind.CLIPBOARD, "old", body = "hello there", ts = now - 30 * day),
            item(FoundKind.CLIPBOARD, "new", body = "hello world", ts = now - day),
        )
        val g = Search.search("hello", items, now = now).single()
        assertEquals(listOf("new", "old"), g.hits.map { it.item.id })
    }

    @Test
    fun betterFieldBeatsRecency() {
        val items = listOf(
            item(FoundKind.TRANSFERS, "body", title = "x.pdf", body = "invoice", ts = now),
            item(FoundKind.TRANSFERS, "title", title = "invoice.pdf", ts = now - 300 * day),
        )
        val g = Search.search("invoice", items, now = now).single()
        assertEquals("title", g.hits.first().item.id)
    }

    @Test
    fun recencyHalvesAfterAWeek() {
        assertEquals(1.0, Search.recency(now, now), 1e-9)
        assertEquals(0.5, Search.recency(now - 7 * day, now), 1e-9)
        assertEquals(0.0, Search.recency(null, now), 1e-9)
    }

    @Test
    fun groupsComeInKindOrderWithTotals() {
        val items = listOf(
            item(FoundKind.HELP, "h", "Pairing a phone"),
            item(FoundKind.DEVICES, "d", "Pixel phone"),
            item(FoundKind.CLIPBOARD, "c1", body = "phone number 1"),
            item(FoundKind.CLIPBOARD, "c2", body = "phone number 2"),
            item(FoundKind.CLIPBOARD, "c3", body = "phone number 3"),
            item(FoundKind.CLIPBOARD, "c4", body = "phone number 4"),
            item(FoundKind.SETTINGS, "s", "Appearance"),
        )
        val groups = Search.search("phone", items, now = now)
        assertEquals(listOf(FoundKind.DEVICES, FoundKind.CLIPBOARD, FoundKind.HELP), groups.map { it.kind })
        val clips = groups[1]
        assertEquals(4, clips.total)
        assertTrue(Search.hasMore(clips))
        assertEquals(3, Search.shown(clips, expanded = false).size)
        assertEquals(4, Search.shown(clips, expanded = true).size)
        assertFalse(Search.hasMore(groups[0]))
    }

    @Test
    fun kindFilterKeepsOnlyChosen() {
        val items = listOf(
            item(FoundKind.DEVICES, "d", "Pixel"),
            item(FoundKind.HELP, "h", "Pixel help"),
        )
        val groups = Search.search("pixel", items, kinds = setOf(FoundKind.HELP), now = now)
        assertEquals(listOf(FoundKind.HELP), groups.map { it.kind })
    }

    @Test
    fun limitCapsHitsNotTotal() {
        val items = (1..10).map { item(FoundKind.CLIPBOARD, "c$it", body = "abc $it") }
        val g = Search.search("abc", items, limit = 4, now = now).single()
        assertEquals(10, g.total)
        assertEquals(4, g.hits.size)
    }

    @Test
    fun untitledShowsSnippetAsTitle() {
        val g = Search.search("milk", listOf(item(FoundKind.MESSAGES, "m", body = "buy\n  milk")), now = now).single()
        assertEquals("buy milk", g.hits[0].title)
        assertEquals("", g.hits[0].detail)
    }

    @Test
    fun titledShowsBodySnippetAsDetail() {
        val g = Search.search("sync", listOf(item(FoundKind.SETTINGS, "s", "Synced folders", body = "Keep a folder")), now = now).single()
        assertEquals("Synced folders", g.hits[0].title)
        assertEquals("Keep a folder", g.hits[0].detail)
    }

    // --- snippets and highlighting ---

    @Test
    fun shortTextIsFlattenedWhole() {
        assertEquals("a b c", Search.snippet(" a\n b\tc ", listOf("b")))
    }

    @Test
    fun snippetCentresOnFirstMatch() {
        val text = "x".repeat(300) + " needle " + "y".repeat(300)
        val s = Search.snippet(text, listOf("needle"), max = 40)
        assertTrue(s.startsWith("…"))
        assertTrue(s.endsWith("…"))
        assertTrue(s.contains("needle"))
        // A quarter of the room (10) before the match, after the "…".
        assertEquals(11, s.indexOf("needle"))
    }

    @Test
    fun snippetWithoutMatchStartsAtTheStart() {
        val s = Search.snippet("a".repeat(200), listOf("zzz"), max = 50)
        assertFalse(s.startsWith("…"))
        assertTrue(s.endsWith("…"))
        assertEquals(51, s.length)
    }

    @Test
    fun snippetAtTheEndIsFull() {
        val text = "y".repeat(300) + " needle"
        val s = Search.snippet(text, listOf("needle"), max = 40)
        assertTrue(s.endsWith("needle"))
        assertEquals(41, s.length)
    }

    @Test
    fun matchRangesPointIntoTheOriginal() {
        // "Café" with a combining accent: the match takes the accent too.
        val text = "Le café noir"
        val spans = Search.matchRanges(text, Search.queryWords("cafe"))
        assertEquals(listOf(MatchSpan(3, 8)), spans)
        assertEquals("café", text.substring(3, 8))
    }

    @Test
    fun matchRangesMergeAndSort() {
        val spans = Search.matchRanges("Paris café Paris", Search.queryWords("paris caf cafe"))
        assertEquals(listOf(MatchSpan(0, 5), MatchSpan(6, 10), MatchSpan(11, 16)), spans)
    }

    @Test
    fun matchRangesEmptyCases() {
        assertTrue(Search.matchRanges("", listOf("a")).isEmpty())
        assertTrue(Search.matchRanges("abc", emptyList()).isEmpty())
        assertTrue(Search.matchRanges("abc", listOf("z")).isEmpty())
    }

    @Test
    fun matchRangesWithSurrogatesAndCase() {
        val text = "😀 HELLO"
        assertEquals(listOf(MatchSpan(3, 8)), Search.matchRanges(text, listOf("hello")))
    }
}
