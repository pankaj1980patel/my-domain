package com.mydomain.android.core

/**
 * Nearby media mode (mdcore `features::nearby`) as the screens show it:
 * the "Nearby · low latency" badge on a running audio, screen or camera
 * view and on the connection map, and the latency line ("12 ms (was
 * 140 ms)"). Plain Kotlin, tested on the JVM.
 */
object NearbyBadge {
    /** A stream's state, from a `nearby-media` event or `nearbyStatus`. */
    data class State(
        val media: Boolean = false,
        /** "lan", "ble" or null. */
        val near: String? = null,
        val lowLatency: Boolean = false,
        val path: String = "",
        val latencyMs: Int? = null,
        val beforeMs: Int? = null,
        val rttMs: Int? = null,
    )

    /** The badge shows while a stream runs on a local link. */
    fun show(s: State?): Boolean = s != null && s.media && s.lowLatency

    /** What the latency line says. */
    sealed class Latency {
        data class Now(val ms: Int) : Latency()
        /** Measured now, and on the link before the last move. */
        data class Was(val ms: Int, val beforeMs: Int) : Latency()
    }

    /** The latency line, when there's a measure (none while nothing runs). */
    fun latency(s: State?): Latency? {
        if (s == null || !s.media) return null
        val now = s.latencyMs ?: return null
        val before = s.beforeMs
        return if (before != null && before != now) Latency.Was(now, before) else Latency.Now(now)
    }

    /** The map's line gets the badge when its stream is nearby. */
    fun onMapLink(nearbyMedia: Boolean, linked: Boolean): Boolean = nearbyMedia && linked
}
