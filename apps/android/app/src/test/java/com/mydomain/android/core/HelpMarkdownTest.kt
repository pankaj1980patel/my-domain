package com.mydomain.android.core

import com.mydomain.android.core.MdBlock.Heading
import com.mydomain.android.core.MdBlock.ListBlock
import com.mydomain.android.core.MdBlock.Para
import com.mydomain.android.core.MdInline.Bold
import com.mydomain.android.core.MdInline.Code
import com.mydomain.android.core.MdInline.Italic
import com.mydomain.android.core.MdInline.Link
import com.mydomain.android.core.MdInline.Text
import java.io.File
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class HelpMarkdownTest {

    private fun topic(name: String, title: String, keywords: String, platforms: String, body: String) =
        HelpMarkdown.parseTopic(name, "---\ntitle: $title\nkeywords: $keywords\nplatforms: $platforms\n---\n$body")!!

    @Test
    fun inlineMarks() {
        assertEquals(
            listOf(
                Text("a "), Bold(listOf(Text("b"))), Text(" "), Italic(listOf(Text("c"))), Text(" "),
                Code("d"), Text(" "), Link("help:x", listOf(Text("e"))), Text(" snake_case_word "),
                Italic(listOf(Text("i"))),
            ),
            HelpMarkdown.parseInline("a **b** *c* `d` [e](help:x) snake_case_word _i_"),
        )
    }

    @Test
    fun unclosedMarksStayText() {
        assertEquals(listOf(Text("2 * 3 and **open [x]")), HelpMarkdown.parseInline("2 * 3 and **open [x]"))
        assertEquals(listOf(Text("a*b")), HelpMarkdown.parseInline("a\\*b"))
    }

    @Test
    fun boldCanHoldALink() {
        assertEquals(
            listOf(Bold(listOf(Text("see "), Link("help:y", listOf(Text("Y")))))),
            HelpMarkdown.parseInline("**see [Y](help:y)**"),
        )
    }

    @Test
    fun blocks() {
        val b = HelpMarkdown.parseBlocks("# T\n\npara one\nline two\n\n- a\n- b\n  more\n1. x\n2. y\nafter\n### Deep\n#### Deeper")
        assertEquals(7, b.size)
        assertEquals(Heading(1, listOf(Text("T"))), b[0])
        assertEquals(Para(listOf(Text("para one line two"))), b[1])
        assertEquals(ListBlock(false, listOf(listOf(Text("a")), listOf(Text("b more")))), b[2])
        assertEquals(ListBlock(true, listOf(listOf(Text("x")), listOf(Text("y")))), b[3])
        assertEquals(Para(listOf(Text("after"))), b[4])
        assertEquals(3, (b[5] as Heading).level)
        assertEquals(3, (b[6] as Heading).level)
    }

    @Test
    fun frontMatter() {
        val (meta, body) = HelpMarkdown.splitFrontMatter("---\r\ntitle: X: y\r\nKeywords: a, b\r\n---\r\nbody\r\n")
        assertEquals(mapOf("title" to "X: y", "keywords" to "a, b"), meta)
        assertEquals("body\n", body)
        assertNull(HelpMarkdown.parseTopic("README.md", "# No front matter"))
    }

    @Test
    fun topicFromFile() {
        val t = topic("docs/help/04-find.md", "Find it", "Locate, Map", "desktop, Android", "Some *text*.")
        assertEquals("find", t.id)
        assertEquals(4, t.order)
        assertEquals(listOf("locate", "map"), t.keywords)
        assertEquals(listOf("desktop", "android"), t.platforms)
        assertEquals("some text.", t.body)
    }

    @Test
    fun searchNeedsEveryWordAndRanksTheTitleFirst() {
        val a = topic("01-a.md", "Calls", "bluetooth", "android", "Talk on the computer.")
        val b = topic("02-b.md", "Audio", "music", "android", "Calls are never sent; bluetooth isn’t used.")
        val c = topic("03-c.md", "Other", "", "desktop", "Nothing here.")
        val all = listOf(c, b, a)
        assertEquals(listOf(a, b, c), HelpMarkdown.search(all, "  "))
        assertEquals(listOf(a, b), HelpMarkdown.search(all, "CALLS"))
        assertEquals(listOf(a, b), HelpMarkdown.search(all, "calls bluetooth"))
        assertEquals(listOf(b), HelpMarkdown.search(all, "calls music"))
        assertEquals(emptyList<HelpTopic>(), HelpMarkdown.search(all, "calls zebra"))
        // A plain apostrophe finds a typographic one.
        assertEquals(listOf(b), HelpMarkdown.search(all, "isn't"))
        assertEquals(listOf(a, b), HelpMarkdown.topicsFor(all, "android"))
    }

    @Test
    fun helpLinks() {
        assertEquals("calls", HelpMarkdown.helpTarget("help:calls"))
        assertNull(HelpMarkdown.helpTarget("https://example.com"))
        assertNull(HelpMarkdown.helpTarget("help:"))
    }

    /** The real topics (repo docs/help) parse, and every help: link in them
     *  points at a topic. Skipped when the folder isn't there. */
    @Test
    fun theShippedTopicsHold() {
        val dir = listOf("../../../docs/help", "../../docs/help", "docs/help").map { File(it) }.firstOrNull { it.isDirectory } ?: return
        val topics = dir.listFiles { f -> f.name.endsWith(".md") }!!.mapNotNull { HelpMarkdown.parseTopic(it.name, it.readText()) }
        assertTrue(topics.size >= 10)
        val ids = topics.map { it.id }.toSet()
        assertEquals(topics.size, ids.size)
        fun links(inlines: List<MdInline>): List<String> = inlines.flatMap {
            when (it) {
                is Link -> listOfNotNull(HelpMarkdown.helpTarget(it.href)) + links(it.children)
                is Bold -> links(it.children)
                is Italic -> links(it.children)
                else -> emptyList()
            }
        }
        for (t in topics) {
            assertTrue(t.id, t.platforms.isNotEmpty() && t.platforms.all { it == "desktop" || it == "android" })
            for (b in t.blocks) {
                val found = when (b) {
                    is Heading -> links(b.inlines)
                    is Para -> links(b.inlines)
                    is ListBlock -> b.items.flatMap { links(it) }
                }
                for (id in found) assertTrue("${t.id} links to $id", id in ids)
            }
        }
    }
}
