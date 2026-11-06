package com.mydomain.android.core

import com.mydomain.android.core.PlaybackPlan.Track
import com.mydomain.android.core.PlaybackPlan.WifiLock
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class PlaybackPlanTest {
    @Test fun theBufferIsThreeTimesTheMinimum() {
        assertEquals(3 * 7680, PlaybackPlan.bufferBytes(7680, 48_000))
        // A tiny minimum still holds four periods.
        assertEquals(480 * 4 * 4, PlaybackPlan.bufferBytes(100, 48_000))
        // The device said nothing: eight periods.
        assertEquals(480 * 4 * 8, PlaybackPlan.bufferBytes(-2, 48_000))
    }

    @Test fun tenMillisecondsAWrite() {
        assertEquals(480, PlaybackPlan.periodFrames(48_000))
        assertEquals(240, PlaybackPlan.periodFrames(24_000))
        assertEquals(220, PlaybackPlan.periodFrames(22_050))
        assertEquals(1, PlaybackPlan.periodFrames(0))
    }

    @Test fun lowLatencyOnlyNearbyAndWhereAndroidHasIt() {
        assertEquals(Track(48_000, true), PlaybackPlan.trackFor(48_000, true, 33))
        assertEquals(Track(48_000, false), PlaybackPlan.trackFor(48_000, false, 33))
        assertEquals(Track(48_000, false), PlaybackPlan.trackFor(48_000, true, 25))
    }

    @Test fun reopenedOnAnotherRateOrMode() {
        val t = Track(48_000, false)
        assertTrue(PlaybackPlan.reopen(null, t))
        assertFalse(PlaybackPlan.reopen(t, t))
        assertTrue(PlaybackPlan.reopen(t, Track(24_000, false)))
        assertTrue(PlaybackPlan.reopen(t, Track(48_000, true)))
        // No stream yet: nothing to open.
        assertFalse(PlaybackPlan.reopen(null, Track(0, false)))
    }

    @Test fun theWifiLockByAndroidVersion() {
        assertEquals(WifiLock.LowLatency, PlaybackPlan.wifiLock(29))
        assertEquals(WifiLock.HighPerf, PlaybackPlan.wifiLock(28))
    }

    @Test fun underrunsSinceTheOutputOpened() {
        assertEquals(3, PlaybackPlan.underruns(2, 5))
        assertEquals(0, PlaybackPlan.underruns(5, 2))
        assertEquals(0, PlaybackPlan.underruns(0, -1))
    }
}
