package com.mydomain.android.core

import com.mydomain.android.core.AudioJitter.Companion.CEIL_MS
import com.mydomain.android.core.AudioJitter.Companion.CONCEAL_MS
import com.mydomain.android.core.AudioJitter.Companion.DRIFT_MAX
import com.mydomain.android.core.AudioJitter.Companion.FLOOR_MS
import com.mydomain.android.core.AudioJitter.Companion.FLOOR_NEARBY_MS
import com.mydomain.android.core.AudioJitter.Companion.GROW_LATE_MS
import com.mydomain.android.core.AudioJitter.Companion.GROW_UNDERRUN_MS
import com.mydomain.android.core.AudioJitter.Companion.START_MS
import com.mydomain.android.core.AudioJitter.Companion.adapt
import com.mydomain.android.core.AudioJitter.Companion.driftRatio
import com.mydomain.android.core.AudioJitter.Event
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/** The same cases as the core's `features::audio_jitter` tests. */
class AudioJitterTest {
    private val rate = 48_000

    /** [ms] of audio, every sample [v]. */
    private fun frame(ms: Int, v: Short, channels: Int = 2): ByteArray {
        val n = rate * ms / 1000 * channels
        val out = ByteArray(n * 2)
        for (i in 0 until n) {
            out[2 * i] = (v.toInt() and 0xff).toByte()
            out[2 * i + 1] = (v.toInt() shr 8).toByte()
        }
        return out
    }

    private fun play(j: AudioJitter, ms: Int): ShortArray {
        val frames = rate * ms / 1000
        val out = ShortArray(frames * 2)
        j.pull(out, frames)
        return out
    }

    @Test fun theTargetGrowsFastAndShrinksSlowly() {
        assertEquals(100, adapt(80, FLOOR_MS, Event.Underrun))
        assertEquals(90, adapt(80, FLOOR_MS, Event.Late))
        assertEquals(CEIL_MS, adapt(195, FLOOR_MS, Event.Underrun))
        assertEquals(70, adapt(80, FLOOR_MS, Event.Stable))
        assertEquals(FLOOR_MS, adapt(65, FLOOR_MS, Event.Stable))
        // Nearby mode only lowers the floor.
        assertEquals(FLOOR_NEARBY_MS, adapt(35, FLOOR_NEARBY_MS, Event.Stable))
        assertEquals(50, adapt(30, FLOOR_NEARBY_MS, Event.Underrun))
    }

    @Test fun steadyAudioShrinksTheTargetToTheFloor() {
        val j = AudioJitter()
        j.setLowLatency(true)
        var seq = 0L
        repeat(12) { j.push(seq++, rate, 2, frame(10, 1000)) }
        repeat(4000) {
            j.push(seq++, rate, 2, frame(10, 1000))
            play(j, 10)
        }
        assertEquals(0, j.underruns)
        assertEquals(FLOOR_NEARBY_MS, j.targetMs)
    }

    @Test fun runningDryConcealsThenGrowsTheTarget() {
        val j = AudioJitter()
        for (seq in 0L until 10) j.push(seq, rate, 2, frame(10, 8000))
        val out = play(j, 100)
        assertEquals(1, j.underruns)
        assertEquals(START_MS + GROW_UNDERRUN_MS, j.targetMs)
        // Just after the stream ran out: the repeat, not silence.
        val dryAt = out.size - 2 * (rate * 20 / 1000)
        assertTrue("${out[dryAt]}", Math.abs(out[dryAt].toInt()) > 1500)
        // A whole concealment later: faded to silence, waiting again.
        val later = play(j, 100)
        assertEquals(0, later[later.size - 2].toInt())
        assertFalse(j.playing())
        val c = j.stats().concealedMs
        assertTrue("$c", c in CONCEAL_MS - 2..CONCEAL_MS + 2)
    }

    @Test fun resumingAfterAGapCrossfades() {
        val j = AudioJitter()
        for (seq in 0L until 9) j.push(seq, rate, 2, frame(10, 8000))
        play(j, 95)
        play(j, 10) // dry: concealing
        j.push(9, rate, 2, frame(10, -8000))
        val out = play(j, 1)
        // The first frames back mix the concealment in: no jump to -8000.
        assertTrue("${out[0]}", out[0] > -6500)
    }

    @Test fun aLateFrameIsDroppedAndGrowsTheTarget() {
        val j = AudioJitter()
        val p = frame(10, 1)
        j.push(5, rate, 2, p)
        j.push(4, rate, 2, p)
        assertEquals(1, j.late)
        assertEquals(START_MS + GROW_LATE_MS, j.targetMs)
    }

    @Test fun driftIsSteeredByATinySpeedChange() {
        assertEquals(1.0, driftRatio(80.0, 80.0), 0.0)
        assertTrue(driftRatio(120.0, 80.0) > 1.0)
        assertTrue(driftRatio(40.0, 80.0) < 1.0)
        assertEquals(1.0 + DRIFT_MAX, driftRatio(10_000.0, 80.0), 1e-12)
        assertEquals(1.0 - DRIFT_MAX, driftRatio(0.0, 80.0), 1e-12)
        // A sender 0.3 % fast: the buffer settles instead of piling up.
        val j = AudioJitter()
        var seq = 0L
        var acc = 0.0
        repeat(20_000) {
            acc += 480.0 * 1.003
            val n = acc.toInt()
            acc -= n
            val pcm = ByteArray(n * 4)
            for (i in 0 until n * 2) pcm[2 * i] = 100
            j.push(seq++, rate, 2, pcm)
            play(j, 10)
        }
        assertEquals(0, j.trimmed)
        assertTrue("${j.bufferedMs()}", j.bufferedMs() < CEIL_MS)
        assertTrue("${j.stats()}", j.stats().driftPpm > 1000)
    }

    @Test fun aPileUpFarPastTheCeilingIsTrimmed() {
        val j = AudioJitter()
        val p = frame(100, 1)
        for (seq in 0L until 4) j.push(seq, rate, 2, p)
        assertEquals(1, j.trimmed)
        assertTrue(j.bufferedMs() <= START_MS + 100)
    }

    @Test fun monoIsDoubledAndANewFormatStartsOver() {
        val j = AudioJitter()
        j.push(0, rate, 1, frame(10, 5, channels = 1))
        assertEquals(10, j.bufferedMs())
        j.push(1, 24_000, 2, ByteArray(24 * 4 * 10))
        assertEquals(24_000, j.rate)
        assertEquals(10, j.bufferedMs())
        j.reset()
        assertEquals(0, j.rate)
    }
}
