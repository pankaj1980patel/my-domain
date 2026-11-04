package com.mydomain.android.core

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class AudioJitterTest {

    /** [ms] of a stereo frame at [rate], every sample [v]. */
    private fun frame(rate: Int, ms: Int, v: Short, channels: Int = 2): ByteArray {
        val n = rate * ms / 1000 * channels
        val out = ByteArray(n * 2)
        for (i in 0 until n) {
            out[2 * i] = (v.toInt() and 0xff).toByte()
            out[2 * i + 1] = (v.toInt() shr 8).toByte()
        }
        return out
    }

    private fun pull(j: AudioJitter, frames: Int): ShortArray {
        val out = ShortArray(frames * 2) { 7 }
        j.pull(out, frames)
        return out
    }

    @Test
    fun itWaitsForTheTargetThenPlays() {
        val j = AudioJitter(80, 200)
        for (seq in 0L until 3L) j.push(seq, 48_000, 2, frame(48_000, 20, 16_384))
        assertFalse("60 ms isn't enough yet", j.playing())
        assertTrue(pull(j, 100).all { it == 0.toShort() })
        j.push(3, 48_000, 2, frame(48_000, 20, 16_384))
        assertTrue(j.playing())
        assertEquals(80, j.bufferedMs())
        assertTrue(pull(j, 960).all { it == 16_384.toShort() })
        assertEquals(60, j.bufferedMs())
        assertEquals(0, j.underruns)
    }

    @Test
    fun runningDryCountsAnUnderrunAndPrimesAgain() {
        val j = AudioJitter(40, 200)
        j.push(0, 48_000, 2, frame(48_000, 40, 1000))
        assertTrue(j.playing())
        val out = pull(j, 48_000 / 10)
        assertEquals(1000.toShort(), out[0])
        assertEquals("the rest is silence", 0.toShort(), out[out.size - 1])
        assertEquals(1, j.underruns)
        assertFalse(j.playing())
        // Still dry: no new underrun counted while waiting.
        pull(j, 480)
        assertEquals(1, j.underruns)
        j.push(1, 48_000, 2, frame(48_000, 40, 1000))
        assertTrue(j.playing())
    }

    @Test
    fun lateFramesAreDropped() {
        val j = AudioJitter(20, 200)
        j.push(5, 48_000, 2, frame(48_000, 20, 1))
        j.push(4, 48_000, 2, frame(48_000, 20, 1))
        j.push(5, 48_000, 2, frame(48_000, 20, 1))
        assertEquals(2, j.late)
        assertEquals(20, j.bufferedMs())
        // A gap (a lost frame) is not late.
        j.push(7, 48_000, 2, frame(48_000, 20, 1))
        assertEquals(2, j.late)
        assertEquals(40, j.bufferedMs())
    }

    @Test
    fun tooMuchIsTrimmedBackToTheTarget() {
        val j = AudioJitter(80, 200)
        for (seq in 0L until 11L) j.push(seq, 48_000, 2, frame(48_000, 20, 1))
        assertEquals(1, j.trimmed)
        assertEquals(80, j.bufferedMs())
        // The newest audio is kept: what's played next is the latest.
        j.push(11, 48_000, 2, frame(48_000, 20, 9))
        val out = pull(j, 48 * 100)
        assertEquals(9.toShort(), out[out.size - 1])
    }

    @Test
    fun aNewFormatStartsOver() {
        val j = AudioJitter(40, 200)
        j.push(0, 48_000, 2, frame(48_000, 60, 1))
        assertTrue(j.playing())
        // The link moved to the relay: 24 kHz, and the counter may restart.
        j.push(0, 24_000, 2, frame(24_000, 20, 1))
        assertEquals(24_000, j.rate)
        assertFalse(j.playing())
        assertEquals(20, j.bufferedMs())
        assertEquals(0, j.late)
    }

    @Test
    fun aResetForgetsTheLastComputer() {
        val j = AudioJitter(20, 200)
        j.push(900, 48_000, 2, frame(48_000, 40, 1))
        j.reset()
        assertEquals(0, j.rate)
        assertEquals(0, j.bufferedMs())
        assertFalse(j.playing())
        // Another computer's count starts low: not late.
        j.push(0, 48_000, 2, frame(48_000, 20, 2))
        assertEquals(0, j.late)
        assertTrue(j.playing())
    }

    @Test
    fun monoIsPlayedOnBothSides() {
        val j = AudioJitter(20, 200)
        j.push(0, 48_000, 1, frame(48_000, 20, 1234, channels = 1))
        assertEquals(20, j.bufferedMs())
        assertTrue(pull(j, 960).all { it == 1234.toShort() })
    }

    @Test
    fun negativeSamplesSurvive() {
        val j = AudioJitter(20, 200)
        j.push(0, 48_000, 2, frame(48_000, 20, (-12_345).toShort()))
        assertTrue(pull(j, 960).all { it == (-12_345).toShort() })
    }

    @Test
    fun aLongRunKeepsItsBufferSmall() {
        // A steady stream, pulled as fast as it comes: never trimmed,
        // never dry, and the buffer stays near the target.
        val j = AudioJitter(80, 200)
        var seq = 0L
        repeat(4) { j.push(seq++, 48_000, 2, frame(48_000, 20, 5)) }
        repeat(500) {
            j.push(seq++, 48_000, 2, frame(48_000, 20, 5))
            pull(j, 960)
        }
        assertEquals(0, j.underruns)
        assertEquals(0, j.trimmed)
        assertEquals(80, j.bufferedMs())
    }
}
