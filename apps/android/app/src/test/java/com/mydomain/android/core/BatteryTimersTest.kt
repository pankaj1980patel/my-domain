package com.mydomain.android.core

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/** The battery audit's timers: the health check's pace, the multicast
 *  lock, the keeper's alarm, the automations' tick and the camera's rate. */
class BatteryTimersTest {
    private val min = 60_000L

    @Test
    fun healthRunsAtOnceWithTheServerTheFirstTime() {
        val s = PowerPolicy.healthStep(now = 1_000_000, lastRun = null, lastServer = null, saving = false)
        assertTrue(s.run)
        assertTrue(s.server)
        assertEquals(PowerPolicy.HEALTH_RESAMPLE_MS, s.waitMs)
    }

    @Test
    fun healthEveryMinuteAndTheServerEveryTen() {
        val t0 = 10_000_000L
        // 30 s after a run: not yet, look again within 15 s.
        var s = PowerPolicy.healthStep(t0 + 30_000, t0, t0, saving = false)
        assertFalse(s.run)
        assertFalse(s.server)
        assertEquals(15_000L, s.waitMs)
        // 55 s: due in 5 s.
        s = PowerPolicy.healthStep(t0 + 55_000, t0, t0, saving = false)
        assertFalse(s.run)
        assertEquals(5_000L, s.waitMs)
        // A minute: a quick run, no server.
        s = PowerPolicy.healthStep(t0 + min, t0, t0, saving = false)
        assertTrue(s.run)
        assertFalse(s.server)
        // Ten minutes since the server: with it.
        s = PowerPolicy.healthStep(t0 + 10 * min, t0 + 9 * min, t0, saving = false)
        assertTrue(s.run)
        assertTrue(s.server)
    }

    @Test
    fun savingBatteryStretchesTheHealthCheck() {
        val t0 = 10_000_000L
        assertFalse(PowerPolicy.healthStep(t0 + 2 * min, t0, t0, saving = true).run)
        val s = PowerPolicy.healthStep(t0 + 5 * min, t0, t0, saving = true)
        assertTrue(s.run)
        assertFalse(s.server)
        assertTrue(PowerPolicy.healthStep(t0 + 30 * min, t0 + 25 * min, t0, saving = true).server)
        assertFalse(PowerPolicy.healthStep(t0 + 29 * min, t0 + 24 * min, t0, saving = true).server)
    }

    @Test
    fun healthWaitIsNeverZeroAndAClockGoingBackIsDue() {
        val t0 = 10_000_000L
        assertTrue(PowerPolicy.healthStep(t0 + 59_999, t0, t0, saving = false).waitMs >= 1_000)
        val back = PowerPolicy.healthStep(t0 - 5_000, t0, t0, saving = false)
        assertTrue(back.run)
        assertTrue(back.server)
    }

    @Test
    fun multicastOnlyOnScreenOrJustAfterANetworkChange() {
        assertTrue(PowerPolicy.multicastWanted(onScreen = true, now = 100, burstUntil = 0))
        assertTrue(PowerPolicy.multicastWanted(onScreen = false, now = 100, burstUntil = 200))
        assertFalse(PowerPolicy.multicastWanted(onScreen = false, now = 200, burstUntil = 200))
        assertFalse(PowerPolicy.multicastWanted(onScreen = false, now = 100, burstUntil = 0))
    }

    @Test
    fun theKeepersUsualCheckDoesntWakeThePhone() {
        assertFalse(NetWatch.checkWakes(0))
        assertTrue(NetWatch.checkWakes(1))
        assertTrue(NetWatch.checkWakes(5))
        assertEquals(NetWatch.CHECK_EVERY_MS, NetWatch.nextCheckMs(0, 0.5))
    }

    @Test
    fun automationsTickQuicklyOnlyWhileALinkIsDown() {
        val b = AutomationBook()
        // Nothing known: rarely.
        assertFalse(b.waiting())
        assertEquals(AutomationBook.TICK_IDLE_MS, b.nextTickMs())
        b.links(listOf("pc"), setOf("pc"), 0)
        assertFalse(b.waiting())
        // Down: quick looks until it counts as away.
        b.links(listOf("pc"), emptySet(), 1_000)
        assertTrue(b.waiting())
        assertEquals(AutomationBook.TICK_WAITING_MS, b.nextTickMs())
        val away = b.links(listOf("pc"), emptySet(), 1_000 + AutomationBook.AWAY_AFTER_MS)
        assertEquals(listOf<Edge>(Edge.Disconnected("pc")), away)
        assertFalse(b.waiting())
        assertEquals(AutomationBook.TICK_IDLE_MS, b.nextTickMs())
        // Back up: still idle.
        b.links(listOf("pc"), setOf("pc"), 100_000)
        assertFalse(b.waiting())
    }

    @Test
    fun aWifiNameStillToComeTicksQuicklyForAWhileOnly() {
        val b = AutomationBook()
        b.wifi(false, null)
        b.wifi(true, null)
        assertTrue(b.waiting())
        repeat(AutomationBook.UNNAMED_LOOKS - 1) { b.wifi(true, null) }
        assertTrue(b.waiting())
        b.wifi(true, null)
        assertFalse(b.waiting())
        // The name came after all: nothing pending.
        val c = AutomationBook()
        c.wifi(false, null)
        c.wifi(true, null)
        c.wifi(true, "Home")
        assertFalse(c.waiting())
    }

    @Test
    fun theCameraRunsNoFasterThanNeeded() {
        val ranges = listOf(15 to 15, 7 to 30, 30 to 30, 15 to 30, 24 to 24, 60 to 60)
        assertEquals(15 to 15, CameraFrames.pickFpsRange(ranges))
        // The widest of the slowest that's fast enough.
        assertEquals(7 to 15, CameraFrames.pickFpsRange(listOf(7 to 15, 15 to 15, 30 to 30)))
        // None fast enough: the fastest.
        assertEquals(5 to 10, CameraFrames.pickFpsRange(listOf(5 to 10, 8 to 8)))
        // Only 30: 30.
        assertEquals(30 to 30, CameraFrames.pickFpsRange(listOf(30 to 30)))
        assertNull(CameraFrames.pickFpsRange(emptyList()))
        assertNull(CameraFrames.pickFpsRange(listOf(0 to 0, 20 to 10)))
    }
}
