package com.mydomain.android.core

/**
 * The plain decisions behind playing a computer's sound (ComputerAudio.kt),
 * kept free of Android so they're tested on the JVM: how big the output's
 * buffer is, how much the player pulls from the jitter buffer per write,
 * when the output has to be opened afresh, and which Wi-Fi lock keeps the
 * radio from batching packets while it plays.
 */
object PlaybackPlan {
    /** The output's buffer, in multiples of the smallest the device takes:
     *  room for a hiccup of the player thread, while the jitter buffer
     *  does the real smoothing. */
    const val BUFFER_TIMES = 3
    /** Milliseconds pulled from the jitter buffer per write. */
    const val PERIOD_MS = 10
    /** Android 8: AudioTrack's low-latency performance mode. */
    const val SDK_LOW_LATENCY_TRACK = 26
    /** Android 10: the low-latency Wi-Fi lock. */
    const val SDK_LOW_LATENCY_WIFI = 29
    /** Android 7: AudioTrack.getUnderrunCount. */
    const val SDK_UNDERRUNS = 24

    /** The output's buffer in bytes, for 16-bit stereo at [rate]:
     *  [BUFFER_TIMES] × the device's minimum ([minBytes]; 0 or less: it
     *  didn't say), at least [PERIOD_MS] × 4. */
    fun bufferBytes(minBytes: Int, rate: Int): Int {
        val period = periodFrames(rate) * 4
        return maxOf(if (minBytes > 0) minBytes * BUFFER_TIMES else period * 8, period * 4)
    }

    /** Stereo frames pulled per write. */
    fun periodFrames(rate: Int): Int = maxOf(rate * PERIOD_MS / 1000, 1)

    /** How the output is opened: its rate, and whether in low-latency
     *  mode (only on a nearby local link, where the stream is steady
     *  enough for a small buffer, and where Android has it). */
    data class Track(val rate: Int, val lowLatency: Boolean)

    fun trackFor(rate: Int, nearbyLowLatency: Boolean, sdk: Int): Track =
        Track(rate, nearbyLowLatency && sdk >= SDK_LOW_LATENCY_TRACK)

    /** The output must be opened afresh: none yet, or another rate or
     *  mode. */
    fun reopen(current: Track?, wanted: Track): Boolean = wanted.rate > 0 && current != wanted

    enum class WifiLock { LowLatency, HighPerf }

    /** The Wi-Fi lock held while playing (stops power save from batching
     *  the packets, which comes out as stutter). */
    fun wifiLock(sdk: Int): WifiLock = if (sdk >= SDK_LOW_LATENCY_WIFI) WifiLock.LowLatency else WifiLock.HighPerf

    /** Underruns of the output since it opened ([base]: the count then);
     *  0 where Android doesn't count them ([now] < 0). */
    fun underruns(base: Int, now: Int): Int = if (now < 0 || base < 0) 0 else maxOf(now - base, 0)
}
