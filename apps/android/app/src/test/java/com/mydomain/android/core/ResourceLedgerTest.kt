package com.mydomain.android.core

import com.mydomain.android.core.ResourceBook.Kind
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class ResourceLedgerTest {
    private var now = 1_000_000L
    private val book = ResourceBook { now }

    @Test
    fun holdAndRelease() {
        book.hold(Kind.SERVICE, "NetService")
        book.hold(Kind.WIFI_LOCK, "audio")
        assertTrue(book.isHeld(Kind.SERVICE, "NetService"))
        assertEquals(2, book.snapshot().size)
        book.release(Kind.WIFI_LOCK, "audio")
        assertFalse(book.isHeld(Kind.WIFI_LOCK, "audio"))
        // Releasing what isn't held is fine.
        book.release(Kind.WIFI_LOCK, "audio")
        assertEquals(listOf("NetService"), book.snapshot().map { it.name })
    }

    @Test
    fun aLockWithATimeoutGoesByItself() {
        book.hold(Kind.WAKE_LOCK, "push", 30_000)
        now += 29_999
        assertTrue(book.isHeld(Kind.WAKE_LOCK, "push"))
        now += 1
        assertFalse(book.isHeld(Kind.WAKE_LOCK, "push"))
    }

    @Test
    fun renewingKeepsWhenItWasFirstTaken() {
        book.hold(Kind.WAKE_LOCK, "audio", 60_000)
        val since = now
        now += 50_000
        book.hold(Kind.WAKE_LOCK, "audio", 60_000)
        now += 50_000
        val e = book.snapshot().single()
        assertEquals(since, e.since)
        assertEquals("wake lock audio · 100 s · 10 s left", book.describe(e))
    }

    @Test
    fun theSameNameOfAnotherKindIsAnotherEntry() {
        book.hold(Kind.WAKE_LOCK, "x")
        now += 1
        book.hold(Kind.WIFI_LOCK, "x")
        assertEquals(listOf(Kind.WAKE_LOCK, Kind.WIFI_LOCK), book.snapshot().map { it.kind })
        assertEquals("wifi lock x · 0 s", book.describe(book.snapshot().last()))
    }
}
