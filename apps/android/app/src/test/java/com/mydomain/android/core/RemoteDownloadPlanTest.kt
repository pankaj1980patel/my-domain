package com.mydomain.android.core

import com.mydomain.android.core.RemoteDownloadPlan.Line
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class RemoteDownloadPlanTest {

    private fun item(id: String, state: String, here: Boolean = true, startedAt: Long = 0) =
        DlItem(id, "n", here, "https://x.io/a.zip", "x.io", "a.zip", state = state, startedAt = startedAt)

    @Test
    fun rowsSayWhatTheDownloadIsDoing() {
        assertEquals(Line.Checking, RemoteDownloadPlan.line(item("a", "checking")))
        assertEquals(Line.Asking, RemoteDownloadPlan.line(item("a", "asking")))
        assertEquals(Line.Queued, RemoteDownloadPlan.line(item("a", "queued")))
        assertEquals(Line.Downloading, RemoteDownloadPlan.line(item("a", "downloading")))
        assertEquals(Line.Requested, RemoteDownloadPlan.line(item("a", "requested", here = false)))
        assertEquals(Line.Requested, RemoteDownloadPlan.line(item("a", "checking", here = false)))
        assertEquals(Line.WaitingThere, RemoteDownloadPlan.line(item("a", "asking", here = false)))
        assertEquals(Line.QueuedThere, RemoteDownloadPlan.line(item("a", "queued", here = false)))
        assertEquals(Line.DownloadingThere, RemoteDownloadPlan.line(item("a", "downloading", here = false)))
    }

    @Test
    fun finishedAndAsking() {
        for (s in listOf("done", "failed", "cancelled", "refused")) assertTrue(item("a", s).finished)
        for (s in listOf("requested", "checking", "asking", "queued", "downloading")) assertFalse(item("a", s).finished)
        assertTrue(item("a", "asking").asking)
        // Asking on the other device isn't a question here.
        assertFalse(item("a", "asking", here = false).asking)
    }

    @Test
    fun questionsComeOldestFirst() {
        val asks = RemoteDownloadPlan.asks(
            listOf(
                item("late", "asking", startedAt = 30),
                item("busy", "downloading", startedAt = 1),
                item("early", "asking", startedAt = 10),
                item("there", "asking", here = false, startedAt = 5),
            ),
        )
        assertEquals(listOf("early", "late"), asks.map { it.id })
    }

    @Test
    fun onlyDevicesThatDownloadLinksAreOffered() {
        val names = mapOf("a" to "Zed", "b" to "anna", "c" to "Old phone", "d" to "Mac")
        val targets = RemoteDownloadPlan.targets(
            listOf("a", "b", "c", "d", "a"),
            supports = { it != "c" },
            linked = { it == "d" },
            name = { names.getValue(it) },
        )
        // Linked first, then by name (any case); the old app never.
        assertEquals(listOf("d", "b", "a"), targets)
        assertEquals(emptyList<String>(), RemoteDownloadPlan.targets(listOf("c"), { false }, { true }, { it }))
    }

    @Test
    fun theLinkInAShare() {
        assertEquals("https://x.io/f.zip", RemoteDownloadPlan.linkIn("Get it: https://x.io/f.zip."))
        assertEquals("https://x.io/b", RemoteDownloadPlan.linkIn("no link here", "https://x.io/b"))
        assertNull(RemoteDownloadPlan.linkIn("ftp://x.io/f.zip"))
        assertNull(RemoteDownloadPlan.linkIn(null))
    }

    @Test
    fun sizeLimitsAndPolicies() {
        assertEquals(0L, RemoteDownloadPlan.nearestMax(0))
        assertEquals(100L, RemoteDownloadPlan.nearestMax(50))
        assertEquals(1024L, RemoteDownloadPlan.nearestMax(1000))
        assertEquals(4096L, RemoteDownloadPlan.nearestMax(4096))
        assertEquals(4096L, RemoteDownloadPlan.nearestMax(100_000))
        assertEquals("never", RemoteDownloadPlan.policy("never"))
        assertEquals("ask", RemoteDownloadPlan.policy("sometimes"))
        assertEquals("ask", RemoteDownloadPlan.policy(null))
    }

    @Test
    fun downloadIdsAreToldApart() {
        assertTrue(RemoteDownloadPlan.isDownloadId("dl0a1b2c3d4e5f6a7b"))
        assertFalse(RemoteDownloadPlan.isDownloadId("0a1b2c3d4e5f6a7b"))
    }

    @Test
    fun aDownloadIsATransferWithItsHost() {
        val b = TransferBook()
        // The download's state arrives before its first progress.
        assertTrue(b.link("dl1", "pixel", "a.zip", true, "x.io", 1))
        assertFalse(b.link("dl1", "pixel", "a.zip", true, "x.io", 2))
        // The server named it.
        assertTrue(b.link("dl1", "pixel", "report.zip", true, "x.io", 3))
        b.progress("dl1", "pixel", "report.zip", 5, 10, true, false, 4)
        assertTrue(b.received("dl1", "pixel", "report.zip", 10, 5))
        val t = b.get("dl1")!!
        assertEquals("x.io", t.link)
        assertEquals("report.zip", t.name)
        val back = TransferBook.decode(TransferBook.encode(b.saved())).single()
        assertEquals("x.io", back.link)
        val row = ActivityFeed.transferRow(back)
        assertEquals("link_downloaded", row.action)
        assertEquals("x.io", row.detail)
        val there = ActivityFeed.transferRow(back.copy(incoming = false))
        assertEquals("link_downloaded_there", there.action)
        // A plain file stays a file.
        assertEquals("file_received", ActivityFeed.transferRow(back.copy(link = null)).action)
    }
}
