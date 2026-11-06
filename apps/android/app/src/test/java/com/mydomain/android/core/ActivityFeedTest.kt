package com.mydomain.android.core

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class ActivityFeedTest {
    private val now = 1_700_000_000_000L

    private fun transfer(id: String, node: String, name: String, incoming: Boolean, ts: Long) = TransferRecord(
        id, node, name, incoming, TransferState.Done, 12_000_000, 12_000_000, started = ts - 1000, ended = ts, via = "wifi_direct",
    )

    private fun clip(id: Long, node: String, text: String, at: Long) = ClipRecord(id, text, false, node, true, at)

    private fun message(id: String, node: String, text: String, at: Long) =
        ChatEntry(id, null, node, text, false, at, ChatStatus.Delivered)

    private fun seeded(): ActivitySources {
        val rec = ActivityRecord()
        rec.push("link_up", "p", "", "quic", now - 500)
        return ActivitySources(
            names = mapOf("p" to "Pixel", "m" to "MacBook"),
            transfers = listOf(transfer("t1", "p", "report.pdf", false, now - 1000)),
            clips = listOf(clip(1, "p", "Café on Rue Cler\nsecond line", now - 2000)),
            messages = listOf(message("m1", "m", "see you at 8", now - 3000)),
            notes = listOf(NoteSource("n1", "Groceries", "milk", now - 4000)),
            log = rec.entries,
        )
    }

    @Test
    fun kindsOfActions() {
        assertEquals("files", ActivityFeed.kindOf("file_sent"))
        assertEquals("clipboard", ActivityFeed.kindOf("clip_in"))
        assertEquals("messages", ActivityFeed.kindOf("msg_failed"))
        assertEquals("calls", ActivityFeed.kindOf("call"))
        assertEquals("connections", ActivityFeed.kindOf("link_down"))
        assertEquals("connections", ActivityFeed.kindOf("key_changed"))
        assertEquals("other", ActivityFeed.kindOf("screen_offer"))
    }

    @Test
    fun mergesEverySourceNewestFirst() {
        val p = ActivityFeed.page(seeded(), ActivityQuery())
        assertEquals(listOf("link_up", "file_sent", "clip_in", "msg_out", "note"), p.items.map { it.action })
        assertFalse(p.more)
        val file = p.items[1]
        assertEquals("report.pdf", file.subject)
        assertEquals(12_000_000L, file.bytes)
        assertEquals("wifi_direct", file.via)
        assertEquals("p", file.node)
        assertEquals("Café on Rue Cler…", p.items[2].subject)
    }

    @Test
    fun filtersByDeviceKindAndWords() {
        val src = seeded()
        val m = ActivityFeed.page(src, ActivityQuery(node = "m"))
        assertEquals(listOf("msg_out"), m.items.map { it.action })
        val kinds = ActivityFeed.page(src, ActivityQuery(kinds = setOf("files", "clipboard")))
        assertEquals(listOf("files", "clipboard"), kinds.items.map { it.kind })
        assertEquals(1, ActivityFeed.page(src, ActivityQuery(text = "CAFE")).items.size)
        // The device's name counts as a word too.
        assertEquals(3, ActivityFeed.page(src, ActivityQuery(text = "pixel")).items.size)
    }

    @Test
    fun hiddenKindsAndClear() {
        val src = seeded()
        val p = ActivityFeed.page(src, ActivityQuery(), hidden = listOf("connections", "files"))
        assertTrue(p.items.none { it.kind == "connections" || it.kind == "files" })
        assertEquals(3, p.items.size)
        // A hidden kind stays hidden even when asked for.
        assertTrue(ActivityFeed.page(src, ActivityQuery(kinds = setOf("files")), hidden = listOf("files")).items.isEmpty())
        assertEquals(3, ActivityFeed.page(src, ActivityQuery(), clearedAt = now - 2500).items.size)
    }

    @Test
    fun theRecordAndClearLeaveHistoriesAlone() {
        val rec = ActivityRecord()
        assertTrue(rec.push("link_up", "p", "", "quic", now))
        assertFalse(rec.push("link_up", "p", "", "relay", now + 1))
        assertTrue(rec.push("link_down", "p", "", "", now + 2))
        assertFalse(rec.push("link_down", "p", "", "", now + 3))
        assertTrue(rec.push("call", "p", "Mum", "ringing", now + 4))
        assertFalse(rec.push("call", "p", "Mum", "ringing", now + 5))
        assertTrue(rec.push("call", "p", "Mum", "missed", now + 6))
        assertEquals(4, rec.entries.size)
        // Never the clipboard's.
        assertFalse(rec.push("clip_in", "p", "hunter2", "", now + 7))
        for (i in 0 until ActivityRecord.MAX + 10) rec.push("ring", "p", "$i", "", now + 10 + i)
        assertEquals(ActivityRecord.MAX, rec.entries.size)
        rec.clear(now + 99_999)
        assertTrue(rec.entries.isEmpty())
        val src = seeded().copy(log = rec.entries)
        assertTrue(ActivityFeed.page(src, ActivityQuery(), clearedAt = rec.clearedAt).items.isEmpty())
        rec.restore()
        assertNull(rec.clearedAt)
        assertEquals(4, ActivityFeed.page(src, ActivityQuery(), clearedAt = rec.clearedAt).items.size)
        assertEquals(1, src.transfers.size)
        assertEquals(1, src.clips.size)
    }

    @Test
    fun recordReadsBack() {
        val rec = ActivityRecord()
        rec.push("wake", "m", "", "", now)
        rec.push("ring", "p", "", "", now + 1)
        rec.setHidden(listOf("other", "bogus", "calls"))
        rec.clear(now + 5)
        rec.push("call", "p", "Mum", "missed", now + 6)
        val back = ActivityRecord.decode(rec.encode())
        assertEquals(rec.entries, back.entries)
        assertEquals(listOf("calls", "other"), back.hidden)
        assertEquals(now + 5, back.clearedAt)
        assertTrue(ActivityRecord.decode("not json").entries.isEmpty())
    }

    @Test
    fun privacyOnlyKeptClipsShowAndOnlyTheirFirstLine() {
        // A copy marked sensitive stays out of the clipboard history
        // (ClipPrivacy.keepHere), and the record refuses clips, so the only
        // clips listed are the history's.
        assertTrue(ClipPrivacy.keepHere(sensitive = true, hide = true))
        val rec = ActivityRecord()
        assertFalse(rec.push("clip_out", "p", "s3cret!", "", now))
        val src = ActivitySources(clips = listOf(clip(1, "p", "first line\nsecond: hunter2", now)), log = rec.entries)
        val p = ActivityFeed.page(src, ActivityQuery())
        assertEquals(1, p.items.size)
        assertFalse(p.items[0].subject.contains("hunter2"))
        assertTrue(ActivityFeed.page(src, ActivityQuery(text = "hunter2")).items.isEmpty())
    }

    @Test
    fun pagesThroughTenThousand() {
        val n = 10_000
        val transfers = ArrayList<TransferRecord>()
        val clips = ArrayList<ClipRecord>()
        val messages = ArrayList<ChatEntry>()
        for (i in 0 until n) {
            val ts = now - (i / 3) * 1000L
            transfers += transfer("t$i", "p", "f.bin", i % 2 == 0, ts)
            clips += clip(i.toLong(), "p", "clip", ts)
            messages += message("m$i", "m", "hi", ts)
        }
        val src = ActivitySources(transfers = transfers, clips = clips.reversed(), messages = messages)
        val started = System.nanoTime()
        val first = ActivityFeed.page(src, ActivityQuery())
        assertEquals(ActivityFeed.PAGE, first.items.size)
        assertTrue(first.more)
        assertTrue("a page took too long", System.nanoTime() - started < 2_000_000_000L)

        val seen = HashSet<String>()
        val all = ArrayList<ActivityRow>()
        var q = ActivityQuery(limit = ActivityFeed.MAX_PAGE)
        while (true) {
            val p = ActivityFeed.page(src, q)
            p.items.forEach { assertTrue("${it.id} twice", seen.add(it.id)) }
            all += p.items
            if (!p.more) break
            val last = p.items.last()
            q = q.copy(beforeTs = last.ts, beforeId = last.id)
        }
        assertEquals(3 * n, all.size)
        for (i in 1 until all.size) {
            assertTrue(ActivityFeed.compare(all[i - 1].ts, all[i - 1].id, all[i].ts, all[i].id) < 0)
        }
        val m = ActivityFeed.page(src, ActivityQuery(node = "m", limit = 10))
        assertTrue(m.items.all { it.action == "msg_out" })
        assertTrue(m.more)
    }

    @Test
    fun groupsByDay() {
        val rows = listOf(
            ActivityRow("a", "other", "ring", null, 3_000, ""),
            ActivityRow("b", "other", "ring", null, 2_000, ""),
            ActivityRow("c", "other", "ring", null, 1_000, ""),
        )
        val days = ActivityFeed.byDay(rows) { if (it >= 2_000) "today" else "before" }
        assertEquals(listOf("today", "before"), days.map { it.first })
        assertEquals(listOf("a", "b"), days[0].second.map { it.id })
    }
}
