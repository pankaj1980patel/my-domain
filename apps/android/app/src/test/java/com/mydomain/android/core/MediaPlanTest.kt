package com.mydomain.android.core

import com.mydomain.android.core.MediaPlan.Op
import com.mydomain.android.core.MediaPlan.Session
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class MediaPlanTest {
    private val spotify = Session("com.spotify.music", "Spotify", "So What", "paused")
    private val youtube = Session("com.google.android.youtube", "YouTube", "Talk", "playing")
    private val podcast = Session("au.com.shiftyjelly.pocketcasts", "Pocket Casts", null, "stopped")

    @Test fun thePickedSessionWinsWhileItsThere() {
        val all = listOf(spotify, youtube, podcast)
        assertEquals(spotify, MediaPlan.pick(all, "com.spotify.music"))
        // Else the one playing, else a paused one, else none.
        assertEquals(youtube, MediaPlan.pick(all, "gone.app"))
        assertEquals(youtube, MediaPlan.pick(all, null))
        assertEquals(spotify, MediaPlan.pick(listOf(podcast, spotify), null))
        assertNull(MediaPlan.pick(listOf(podcast), null))
        assertNull(MediaPlan.pick(emptyList(), "x"))
    }

    @Test fun commandsMapOntoTheSession() {
        assertEquals(Op.Pause, MediaPlan.op("play_pause", null, null, playing = true, volumeMax = 15))
        assertEquals(Op.Play, MediaPlan.op("play_pause", null, null, playing = false, volumeMax = 15))
        assertEquals(Op.Next, MediaPlan.op("next", null, null, true, null))
        assertEquals(Op.Previous, MediaPlan.op("previous", null, null, true, null))
        assertEquals(Op.SeekTo(61_500), MediaPlan.op("seek", 61_500, null, true, null))
        assertNull(MediaPlan.op("seek", null, null, true, null))
        assertNull(MediaPlan.op("seek", -5, null, true, null))
        assertEquals(Op.Adjust(1), MediaPlan.op("volume_up", null, null, true, null))
        assertEquals(Op.Adjust(-1), MediaPlan.op("volume_down", null, null, true, null))
        // Brought into the session's own scale.
        assertEquals(Op.SetVolume(15), MediaPlan.op("volume_set", 900, null, true, 15))
        assertEquals(Op.SetVolume(7), MediaPlan.op("volume_set", 7, null, true, 15))
        assertNull(MediaPlan.op("volume_set", 7, null, true, null))
        assertEquals(Op.Choose("a"), MediaPlan.op("session", null, "a", true, null))
        assertNull(MediaPlan.op("session", null, " ", true, null))
        assertNull(MediaPlan.op("rm -rf", null, null, true, null))
        for (a in MediaPlan.ACTIONS - setOf("seek", "volume_set", "session")) {
            assert(MediaPlan.op(a, null, null, true, 15) != null) { a }
        }
    }

    @Test fun thePositionRunsOnWhilePlaying() {
        assertEquals(2_500L, MediaPlan.positionNow(1_000, 5_000, true, 1_500))
        assertEquals(5_000L, MediaPlan.positionNow(1_000, 5_000, true, 60_000))
        assertEquals(1_000L, MediaPlan.positionNow(1_000, 5_000, false, 60_000))
        assertEquals(61_000L, MediaPlan.positionNow(1_000, null, true, 60_000))
        assertEquals(1_000L, MediaPlan.positionNow(1_000, 5_000, true, -10))
        assertNull(MediaPlan.positionNow(null, 5_000, true, 10))
    }

    @Test fun timesReadLikeAClock() {
        assertEquals("0:00", MediaPlan.clock(0))
        assertEquals("3:07", MediaPlan.clock(187_400))
        assertEquals("1:02:05", MediaPlan.clock(3_725_000))
        assertEquals("0:00", MediaPlan.clock(-4))
        assertEquals(30_000L, MediaPlan.seekTarget(0.5f, 60_000))
        assertEquals(60_000L, MediaPlan.seekTarget(1.7f, 60_000))
    }

    @Test fun statesAndPictureSizes() {
        assertEquals("playing", MediaPlan.stateWord(3))
        assertEquals("playing", MediaPlan.stateWord(6))
        assertEquals("paused", MediaPlan.stateWord(2))
        assertEquals("stopped", MediaPlan.stateWord(1))
        assertEquals(256 to 144, MediaPlan.fit(1920, 1080, 256))
        assertEquals(100 to 50, MediaPlan.fit(100, 50, 256))
        assertEquals(0 to 0, MediaPlan.fit(0, 50, 256))
    }
}
