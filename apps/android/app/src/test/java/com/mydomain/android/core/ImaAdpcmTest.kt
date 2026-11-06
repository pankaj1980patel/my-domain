package com.mydomain.android.core

import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class ImaAdpcmTest {
    // The core's vectors (mdcore features::audio_codec VECTOR_*): change
    // them only together.
    private val vectorPcm = shortArrayOf(
        0, 1000, 3000, 6000, 8000, 7000, 3000, -2000, -8000, -12000, -9000, -3000, 0, 32767, -32768, 100,
    )
    private val vectorAdpcm = intArrayOf(16, 0, 0, 0, 0, 0, 112, 119, 119, 247, 255, 48, 113, 31)
        .map { it.toByte() }.toByteArray()
    private val vectorDecoded = shortArrayOf(
        0, 11, 41, 104, 240, 533, 1164, -193, -3103, -9339, -8448, -2775, -566, 9479, -12057, -2825,
    )

    private fun le(s: ShortArray): ByteArray {
        val out = ByteArray(s.size * 2)
        for (i in s.indices) {
            out[2 * i] = s[i].toByte()
            out[2 * i + 1] = (s[i].toInt() shr 8).toByte()
        }
        return out
    }

    private fun samples(b: ByteArray): ShortArray =
        ShortArray(b.size / 2) { ((b[2 * it].toInt() and 0xff) or (b[2 * it + 1].toInt() shl 8)).toShort() }

    @Test fun theSharedVectorCodesTheSameAsTheCore() {
        val got = ImaAdpcm.Encoder().encode(le(vectorPcm), 1)
        assertArrayEquals(vectorAdpcm, got)
        assertArrayEquals(vectorDecoded, samples(ImaAdpcm.decode(vectorAdpcm, 1)!!))
    }

    @Test fun aSineRoundTripsCloselyAndIsAQuarterTheSize() {
        val enc = ImaAdpcm.Encoder()
        var worst = 0
        var t = 0
        for (frame in 0 until 20) {
            val s = ShortArray(960 * 2)
            for (i in 0 until 960) {
                val v = (Math.sin(t * 440.0 * 2.0 * Math.PI / 48_000.0) * 12_000.0).toInt().toShort()
                t++
                s[2 * i] = v
                s[2 * i + 1] = (v / 2).toShort()
            }
            val pcm = le(s)
            val a = enc.encode(pcm, 2)
            assertEquals(ImaAdpcm.encodedLen(960, 2), a.size)
            assertTrue(a.size * 4 <= pcm.size + 40)
            val back = samples(ImaAdpcm.decode(a, 2)!!)
            if (frame > 0) for (i in s.indices) worst = maxOf(worst, Math.abs(s[i] - back[i]))
        }
        assertTrue("error $worst", worst < 1200)
    }

    @Test fun damagedPayloadsAreRefused() {
        val a = ImaAdpcm.Encoder().encode(le(shortArrayOf(1, 2, 3, 4)), 2)
        assertNull(ImaAdpcm.decode(a.copyOf(a.size - 1), 2))
        assertNull(ImaAdpcm.decode(a, 1))
        assertNull(ImaAdpcm.decode(ByteArray(0), 1))
        val bad = a.copyOf()
        bad[4] = 200.toByte()
        assertNull(ImaAdpcm.decode(bad, 2))
        val odd = ImaAdpcm.Encoder().encode(le(shortArrayOf(1, 2, 3)), 1)
        assertEquals(3, samples(ImaAdpcm.decode(odd, 1)!!).size)
    }

    @Test fun redPacksAndUnpacks() {
        val p = ImaAdpcm.packRed("now".toByteArray(), 1, "before".toByteArray())
        val r = ImaAdpcm.unpackRed(p)
        assertNotNull(r)
        assertEquals("now", String(r!!.primary))
        assertEquals(1, r.back)
        assertEquals("before", String(r.redundant))
        assertNull(ImaAdpcm.unpackRed(p.copyOf(3)))
        assertNull(ImaAdpcm.unpackRed(ImaAdpcm.packRed("x".toByteArray(), 0, "y".toByteArray())))
        assertNull(ImaAdpcm.unpackRed(ByteArray(0)))
    }
}
