package com.mydomain.android.core

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class ChatLogTest {

    private fun sent(id: String, to: String, text: String, at: Long, status: ChatStatus = ChatStatus.Sent) =
        ChatEntry(id, null, to, text, false, at, status)

    private fun got(id: String, from: String, text: String, at: Long) =
        ChatEntry(id, from, null, text, false, at, ChatStatus.Received)

    private fun ids(p: ChatPage) = p.entries.map { it.id }

    @Test
    fun keepsTheNewestPerDevice() {
        val log = ChatLog()
        for (i in 0 until ChatLog.MAX_PER_DEVICE + 5) assertTrue(log.add(sent("$i", "p", "hi", i.toLong())))
        log.add(got("q1", "q", "hello", 1))
        assertEquals(ChatLog.MAX_PER_DEVICE + 1, log.total)
        val p = log.page("p", null, ChatLog.MAX_PAGE)
        assertEquals(ChatLog.MAX_PER_DEVICE, p.kept)
        assertEquals("${ChatLog.MAX_PER_DEVICE + 4}", p.entries.last().id)
        // The oldest five went.
        assertEquals("5", log.snapshot().first { it.node == "p" }.id)
        assertEquals(1, log.page("q", null, 10).kept)
    }

    @Test
    fun keepsTheNewestOverAllDevices() {
        val log = ChatLog()
        val devices = ChatLog.MAX_TOTAL / ChatLog.MAX_PER_DEVICE + 1
        var at = 0L
        for (d in 0 until devices) {
            for (i in 0 until ChatLog.MAX_PER_DEVICE) log.add(sent("$d-$i", "n$d", "x", ++at))
        }
        assertEquals(ChatLog.MAX_TOTAL, log.total)
        // The first device's (the oldest) went entirely.
        assertEquals(0, log.page("n0", null, 10).kept)
        assertEquals(ChatLog.MAX_PER_DEVICE, log.page("n1", null, 10).kept)
    }

    @Test
    fun longTextIsCutAndBlankOrRepeatedOnesAreNotKept() {
        val log = ChatLog()
        val long = "é".repeat(ChatLog.MAX_TEXT) // two bytes each
        assertTrue(log.add(got("a", "p", long, 1)))
        val e = log.page("p", null, 1).entries.single()
        assertTrue(e.truncated)
        assertEquals(ChatLog.MAX_TEXT / 2, e.text.length)
        assertFalse(log.add(got("b", "p", "   ", 2)))
        assertFalse(log.add(got("a", "p", "again", 3)))
        assertFalse(log.add(got("c", "", "no device", 4)))
        assertEquals(1, log.total)
    }

    @Test
    fun pagesBackFromTheNewest() {
        val log = ChatLog()
        for (i in 0 until 250) log.add(sent("m$i", "p", "hi", i.toLong()))
        var p = log.page("p", null, 100)
        assertEquals(100, p.entries.size)
        assertEquals("m150", p.entries.first().id)
        assertEquals("m249", p.entries.last().id)
        assertTrue(p.more)
        p = log.page("p", "m150", 100)
        assertEquals(listOf("m50", "m149"), listOf(p.entries.first().id, p.entries.last().id))
        assertTrue(p.more)
        p = log.page("p", "m50", 100)
        assertEquals(50, p.entries.size)
        assertFalse(p.more)
        assertEquals(250, p.kept)
        // Gone since: nothing, rather than a repeat of the newest.
        assertTrue(log.page("p", "gone", 100).entries.isEmpty())
        assertTrue(log.page("nobody", null, 100).entries.isEmpty())
    }

    @Test
    fun aMessageStampedEarlierGoesInItsPlace() {
        val log = ChatLog()
        log.add(sent("a", "p", "1", 10))
        log.add(sent("c", "p", "3", 30))
        log.add(got("b", "p", "2", 20))
        assertEquals(listOf("a", "b", "c"), ids(log.page("p", null, 10)))
    }

    @Test
    fun statusesChangeById() {
        val log = ChatLog()
        log.add(sent("a", "p", "hi", 1, ChatStatus.Queued))
        log.add(got("b", "p", "yo", 2))
        assertTrue(log.setStatus("a", ChatStatus.Sent))
        assertTrue(log.setStatus("a", ChatStatus.Delivered))
        // Delivered stays delivered (a late send result doesn't undo it).
        assertFalse(log.setStatus("a", ChatStatus.Sent))
        assertFalse(log.setStatus("a", ChatStatus.Failed))
        // A received one never changes; an unknown id is nothing.
        assertFalse(log.setStatus("b", ChatStatus.Delivered))
        assertFalse(log.setStatus("x", ChatStatus.Delivered))
        val p = log.page("p", null, 10)
        assertEquals(listOf(ChatStatus.Delivered, ChatStatus.Received), p.entries.map { it.status })
    }

    @Test
    fun clearingForgetsOneDevice() {
        val log = ChatLog()
        log.add(sent("a", "p", "hi", 1))
        log.add(got("b", "q", "yo", 2))
        assertTrue(log.clear("p"))
        assertFalse(log.clear("p"))
        assertEquals(1, log.total)
        assertEquals(1, log.page("q", null, 10).kept)
        log.clearAll()
        assertEquals(0, log.total)
        assertTrue(log.snapshot().isEmpty())
    }

    @Test
    fun savedAndReadBack() {
        val log = ChatLog()
        log.add(sent("a", "p", "hello \"there\"\n", 1))
        log.add(got("b", "p", "Café ☕", 2))
        log.add(sent("c", "q", "later", 3, ChatStatus.Queued))
        log.setStatus("a", ChatStatus.Delivered)
        val back = ChatLog.read(log.encode())
        assertEquals(3, back.total)
        val p = back.page("p", null, 10)
        assertEquals(listOf("a", "b"), ids(p))
        assertEquals(ChatStatus.Delivered, p.entries[0].status)
        assertEquals("hello \"there\"\n", p.entries[0].text)
        assertEquals("Café ☕", p.entries[1].text)
        assertEquals("p", p.entries[1].node)
        assertTrue(p.entries[1].incoming)
        // Still waiting for a link when the app closed: it never went.
        assertEquals(ChatStatus.Failed, back.page("q", null, 10).entries.single().status)
        // Unreadable text, or entries that make no sense, are none.
        assertEquals(0, ChatLog.read("not json").total)
        assertEquals(0, ChatLog.read("""[{"id":"x","text":"hi","status":"nope"}]""").total)
    }

    @Test
    fun readsTheDesktopsLayout() {
        val json = """[{"id":"m1","from_node":null,"to_node":"p","text":"hi","truncated":false,"at":1700000000000.0,"status":"delivered"}]"""
        val e = ChatLog.read(json).snapshot().single()
        assertEquals(ChatEntry("m1", null, "p", "hi", false, 1_700_000_000_000L, ChatStatus.Delivered), e)
    }

    @Test
    fun olderOnesFromTheFileGoUnderNewerOnes() {
        val log = ChatLog()
        log.add(got("new", "p", "kept while reading", 50))
        log.add(sent("a", "p", "newer copy", 10, ChatStatus.Delivered))
        log.addOlder(listOf(sent("a", "p", "older copy", 10), got("old", "p", "from the file", 5)))
        val p = log.page("p", null, 10)
        assertEquals(listOf("old", "a", "new"), ids(p))
        assertEquals(ChatStatus.Delivered, p.entries[1].status)
    }

    @Test
    fun theSnapshotHoldsEveryDevice() {
        val log = ChatLog()
        log.add(sent("a", "p", "hello", 1))
        log.add(got("b", "q", "there", 2))
        val all = log.snapshot()
        assertEquals(2, all.size)
        assertTrue(all.any { it.node == "p" && !it.incoming })
        assertTrue(all.any { it.node == "q" && it.incoming })
    }
}
