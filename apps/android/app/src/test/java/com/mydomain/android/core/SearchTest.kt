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

    // --- sources ---

    private fun entry(id: String, node: String, incoming: Boolean, text: String, at: Long) = ChatEntry(
        id, if (incoming) node else null, if (incoming) null else node, text, false, at,
        if (incoming) ChatStatus.Received else ChatStatus.Delivered,
    )

    @Test
    fun messagesMergeSavedAndSessionById() {
        val saved = listOf(
            entry("a", "p", false, "dinner at the café?", now - 3 * day),
            entry("b", "p", true, "Cafe is fine", now - day),
        )
        // "b" is also on screen this session: its live bubble wins. The
        // other live one isn't saved.
        val live = listOf(
            SessionMessage(7, "b", "p", true, "Cafe is fine", now - day),
            SessionMessage(8, null, "m", false, "café later", now),
        )
        val items = Search.messageFindables(saved, live, sent = "Sent", received = "Received")
        assertEquals(3, items.size)
        assertTrue(items.all { it.kind == FoundKind.MESSAGES && it.title.isEmpty() })
        val a = items.single { it.body.startsWith("dinner") }
        assertEquals(saved[0].bubble.toString(), a.id)
        assertEquals("p", a.node)
        assertEquals("Sent", a.tag)
        assertEquals(now - 3 * day, a.ts)
        assertEquals(listOf("7", "8"), items.filter { it.body != a.body }.map { it.id })
        assertEquals("Received", items.single { it.id == "7" }.tag)

        val g = Search.search("cafe", items, now = now).single()
        assertEquals(3, g.total)
        assertEquals(listOf("8", "7", a.id), g.hits.map { it.item.id })
        // The text is the line (no title).
        assertEquals("café later", g.hits[0].title)
    }

    @Test
    fun messagesWithNoHistoryAreTheSessionOnes() {
        val items = Search.messageFindables(
            emptyList(), listOf(SessionMessage(1, "x", "", true, "hi", now)), sent = "S", received = "R",
        )
        assertEquals(listOf("1"), items.map { it.id })
        assertNull(items[0].node)
    }

    @Test
    fun savedBubbleIdIsStable() {
        val e = entry("msg-42", "p", true, "x", now)
        assertEquals(e.bubble, e.copy(text = "y").bubble)
        assertEquals("msg-42".hashCode().toLong(), e.bubble)
    }

    @Test
    fun noteTitleFallsBackToFirstLine() {
        assertEquals("Groceries", Search.noteTitle("  Groceries ", "milk"))
        assertEquals("Trip plan", Search.noteTitle(" ", "\n   \n  Trip plan  \nbook"))
        assertEquals("", Search.noteTitle("", "  \n "))
    }

    @Test
    fun notesMapAndRank() {
        val notes = listOf(
            NoteSource("n1", "Groceries", "milk\nbread\nCafé beans", now - 2 * day, by = "m"),
            NoteSource("n2", "", "Trip plan\nbook the café", now),
            NoteSource("n3", "Groceries (conflict from Pixel)", "milk\neggs", now - day, by = "p", conflict = true),
            NoteSource("n4", "Blank", "", 0),
        )
        val items = Search.noteFindables(notes, conflictTag = "Conflict copy")
        assertEquals(listOf("n1", "n2", "n3", "n4"), items.map { it.id })
        assertTrue(items.all { it.kind == FoundKind.NOTES })
        assertEquals("Trip plan", items[1].title)
        assertEquals("m", items[0].node)
        assertNull(items[1].node)
        assertEquals(now - 2 * day, items[0].ts)
        assertNull(items[3].ts)
        assertNull(items[0].tag)
        assertEquals("Conflict copy", items[2].tag)

        // Title words first, newer first among equals; the text as detail.
        val g = Search.search("groceries", items, now = now).single()
        assertEquals(FoundKind.NOTES, g.kind)
        assertEquals(listOf("n3", "n1"), g.hits.map { it.item.id })
        assertEquals("milk bread Café beans", g.hits[1].detail)
        // In the text, accents ignored.
        assertEquals(listOf("n2", "n1"), Search.search("cafe", items, now = now).single().hits.map { it.item.id })
        // A first line standing in for the title counts as a title.
        val trip = Search.search("trip", items, now = now).single().hits.single()
        assertEquals("Trip plan", trip.title)
        assertEquals("Trip plan book the café", trip.detail)
    }

    @Test
    fun notesGroupComesAfterMessages() {
        val items = listOf(
            item(FoundKind.CLIPBOARD, "c", body = "plan"),
            item(FoundKind.NOTES, "n", title = "plan"),
            item(FoundKind.MESSAGES, "m", body = "plan"),
        )
        assertEquals(
            listOf(FoundKind.MESSAGES, FoundKind.NOTES, FoundKind.CLIPBOARD),
            Search.search("plan", items, now = now).map { it.kind },
        )
    }
}
