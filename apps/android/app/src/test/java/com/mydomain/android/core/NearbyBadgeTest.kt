package com.mydomain.android.core

import com.mydomain.android.core.NearbyBadge.Latency
import com.mydomain.android.core.NearbyBadge.State
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class NearbyBadgeTest {
    @Test fun theBadgeShowsWhileAStreamRunsOnALocalLink() {
        assertTrue(NearbyBadge.show(State(media = true, lowLatency = true)))
        assertFalse(NearbyBadge.show(State(media = false, lowLatency = true)))
        assertFalse(NearbyBadge.show(State(media = true, lowLatency = false, near = "lan")))
        assertFalse(NearbyBadge.show(null))
    }

    @Test fun theLatencyLineSaysWhatItWasBeforeTheMove() {
        assertEquals(Latency.Was(12, 140), NearbyBadge.latency(State(media = true, latencyMs = 12, beforeMs = 140)))
        assertEquals(Latency.Now(12), NearbyBadge.latency(State(media = true, latencyMs = 12)))
        assertEquals(Latency.Now(12), NearbyBadge.latency(State(media = true, latencyMs = 12, beforeMs = 12)))
        assertNull(NearbyBadge.latency(State(media = true)))
        assertNull(NearbyBadge.latency(State(media = false, latencyMs = 12)))
    }

    @Test fun theMapLineGetsTheBadgeOnlyWhileLinked() {
        assertTrue(NearbyBadge.onMapLink(nearbyMedia = true, linked = true))
        assertFalse(NearbyBadge.onMapLink(nearbyMedia = true, linked = false))
        assertFalse(NearbyBadge.onMapLink(nearbyMedia = false, linked = true))
    }
}
