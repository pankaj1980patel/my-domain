package com.mydomain.android.core

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class AudioPlanTest {

    @Test
    fun oldPhonesAreNotOffered() {
        assertEquals(AudioPlan.Step.Unsupported, AudioPlan.nextStep(28, false, true))
    }

    @Test
    fun aScreenShareComesFirst() {
        assertEquals(AudioPlan.Step.ScreenBusy, AudioPlan.nextStep(33, true, true))
    }

    @Test
    fun theMicrophonePermissionThenAndroidsPrompt() {
        assertEquals(AudioPlan.Step.AskPermission, AudioPlan.nextStep(29, false, false))
        assertEquals(AudioPlan.Step.AskConsent, AudioPlan.nextStep(34, false, true))
    }

    @Test
    fun twentyMillisecondsOfStereo() {
        assertEquals(3840, AudioPlan.frameBytes(48_000))
        assertEquals(1920, AudioPlan.frameBytes(24_000))
        // 44.1 kHz: 882 whole samples per channel.
        assertEquals(882 * 4, AudioPlan.frameBytes(44_100))
        assertEquals(0, AudioPlan.frameBytes(48_000) % 4)
    }

    @Test
    fun theRateFollowsTheCore() {
        assertEquals(24_000, AudioPlan.rateOf(24_000))
        assertEquals(48_000, AudioPlan.rateOf(null))
        assertEquals(48_000, AudioPlan.rateOf(0))
        assertEquals(48_000, AudioPlan.rateOf(12_345))
    }

    @Test
    fun itStopsOnlyAfterTheGraceAndTheIdleTime() {
        assertFalse(AudioPlan.shouldStop(now = 10_000, graceUntil = 20_000, lastListened = 0, idleMs = 5_000))
        assertFalse(AudioPlan.shouldStop(now = 24_000, graceUntil = 20_000, lastListened = 21_000, idleMs = 5_000))
        assertTrue(AudioPlan.shouldStop(now = 27_000, graceUntil = 20_000, lastListened = 21_000, idleMs = 5_000))
    }
}
