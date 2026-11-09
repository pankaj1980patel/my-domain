package com.mydomain.android.core

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class EngineBackoffTest {
    @Test
    fun startsAtTwoSecondsAndDoubles() {
        assertEquals(2_000L, EngineBackoff.delayMs(1))
        assertEquals(4_000L, EngineBackoff.delayMs(2))
        assertEquals(8_000L, EngineBackoff.delayMs(3))
        assertEquals(16_000L, EngineBackoff.delayMs(4))
    }

    @Test
    fun neverMoreThanFiveMinutes() {
        assertEquals(EngineBackoff.MAX_MS, EngineBackoff.delayMs(9))
        assertEquals(EngineBackoff.MAX_MS, EngineBackoff.delayMs(1_000))
        assertEquals(EngineBackoff.MAX_MS, EngineBackoff.delayMs(Int.MAX_VALUE))
    }

    @Test
    fun neverShrinksAndNeverZero() {
        var last = 0L
        for (n in -2..40) {
            val d = EngineBackoff.delayMs(n)
            assertTrue("$n: $d", d >= last && d > 0)
            last = d
        }
    }
}
