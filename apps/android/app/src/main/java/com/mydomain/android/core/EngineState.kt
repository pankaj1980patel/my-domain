package com.mydomain.android.core

/**
 * Where the Rust engine stands in this process ([Core.engineState]): what
 * NetService, the push handler and the screens read to know whether there
 * is an engine to talk to, and why not.
 */
sealed interface EngineState {
    /** Being started (or started again after a failure). */
    data object Starting : EngineState

    /** Running: [Core.engine] returns it. */
    data object Ready : EngineState

    /** The last start failed with [reason]; [attempt] starts have failed
     *  in a row and the next is in [retryInMs]. */
    data class Failed(val reason: String, val attempt: Int, val retryInMs: Long) : EngineState
}

/** How long NetService waits before starting a failed engine again: 2 s,
 *  doubling each time, at most 5 minutes. */
object EngineBackoff {
    const val FIRST_MS = 2_000L
    const val MAX_MS = 5 * 60_000L

    /** The wait after [failures] failed starts in a row (1 = the first). */
    fun delayMs(failures: Int): Long {
        if (failures <= 1) return FIRST_MS
        val shift = (failures - 1).coerceAtMost(20)
        return (FIRST_MS shl shift).coerceAtMost(MAX_MS)
    }
}
