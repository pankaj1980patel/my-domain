package com.mydomain.android.core

import com.mydomain.android.core.ReadLaterLinks.Item
import com.mydomain.android.core.ReadLaterLinks.Shared
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class ReadLaterLinksTest {
    @Test
    fun aBrowserShareGivesTheLinkAndTheSubjectAsTitle() {
        assertEquals(
            Shared("https://example.com/post", "A good post"),
            ReadLaterLinks.fromShare("https://example.com/post", "A good post"),
        )
    }

    @Test
    fun wordsAroundTheLinkAreTheTitle() {
        assertEquals(
            Shared("https://news.site/a?id=1", "Big story"),
            ReadLaterLinks.fromShare("Big story - https://news.site/a?id=1", null),
        )
        assertEquals(
            Shared("https://news.site/a", "Read this"),
            ReadLaterLinks.fromShare("Read this:\nhttps://news.site/a.", ""),
        )
    }

    @Test
    fun noTitleWhenThereIsNone() {
        assertEquals(Shared("http://x.io", null), ReadLaterLinks.fromShare("http://x.io", null))
        // A subject that's the link itself isn't a title.
        assertEquals(Shared("http://x.io", null), ReadLaterLinks.fromShare("http://x.io", "http://x.io"))
        // Too long to be a title (a quoted paragraph): left out.
        val long = "word ".repeat(100)
        assertEquals(Shared("https://x.io/p", null), ReadLaterLinks.fromShare("$long https://x.io/p", null))
    }

    @Test
    fun theLinkMayBeInTheSubject() {
        assertEquals(Shared("https://x.io/s", null), ReadLaterLinks.fromShare("", "https://x.io/s"))
    }

    @Test
    fun onlyWebLinks() {
        assertNull(ReadLaterLinks.fromShare("just some words", "a subject"))
        assertNull(ReadLaterLinks.fromShare("ftp://x.io/file", null))
        assertNull(ReadLaterLinks.fromShare(null, null))
        assertNull(ReadLaterLinks.firstLink("https:///nohost"))
    }

    @Test
    fun punctuationAfterALinkIsNotPartOfIt() {
        assertEquals(
            "https://en.wikipedia.org/wiki/Rust_(programming_language)",
            ReadLaterLinks.firstLink("(see https://en.wikipedia.org/wiki/Rust_(programming_language))"),
        )
        assertEquals("https://x.io/a", ReadLaterLinks.firstLink("At https://x.io/a, then."))
        assertEquals("HTTPS://X.io", ReadLaterLinks.firstLink("HTTPS://X.io!"))
    }

    @Test
    fun unreadFirstFilteredAndNewest() {
        val items = listOf(
            Item("a", "https://a.io/rust", "Learning Rust", "a.io", read = false, savedAt = 10),
            Item("b", "https://b.io/x", "b.io", "b.io", read = true, savedAt = 30),
            Item("c", "https://c.io/zebra", "Zebras", "c.io", read = false, savedAt = 20),
        )
        assertEquals(listOf("c", "a"), ReadLaterLinks.visible(items, showRead = false, query = "").map { it.id })
        assertEquals(listOf("b", "c", "a"), ReadLaterLinks.visible(items, showRead = true, query = "").map { it.id })
        assertEquals(listOf("a"), ReadLaterLinks.visible(items, showRead = true, query = "rust LEARNING").map { it.id })
        assertEquals(listOf("b"), ReadLaterLinks.visible(items, showRead = true, query = "b.io").map { it.id })
        assertEquals(emptyList<Item>(), ReadLaterLinks.visible(items, showRead = false, query = "b.io"))
    }
}
