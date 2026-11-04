package com.mydomain.android.core

/**
 * The plain decisions behind playing this phone's audio on a computer
 * (PhoneAudio.kt), kept free of Android so they're tested on the JVM.
 */
object AudioPlan {
    /** Android 10: the first with playback capture. Older phones don't
     *  offer the feature at all. */
    const val MIN_SDK = 29
    /** Milliseconds of audio per frame sent. */
    const val FRAME_MS = 20
    const val CHANNELS = 2
    /** The rate when the core gave none (a direct link's). */
    const val DEFAULT_RATE = 48_000
    /** Rates the core accepts (features::audio_share::RATES). */
    val RATES = setOf(16_000, 22_050, 24_000, 44_100, 48_000)

    /** What starting needs next. */
    enum class Step { Unsupported, ScreenBusy, AskPermission, AskConsent }

    fun nextStep(sdk: Int, screenSharing: Boolean, hasRecordAudio: Boolean): Step = when {
        sdk < MIN_SDK -> Step.Unsupported
        // One capture at a time: a second MediaProjection would end the first.
        screenSharing -> Step.ScreenBusy
        !hasRecordAudio -> Step.AskPermission
        else -> Step.AskConsent
    }

    /** The rate to capture at: the one the core gave for the link, else
     *  the direct rate. */
    fun rateOf(offered: Int?): Int = offered?.takeIf { it in RATES } ?: DEFAULT_RATE

    /** Bytes in one frame of 16-bit PCM: whole samples on every channel. */
    fun frameBytes(rate: Int, channels: Int = CHANNELS, ms: Int = FRAME_MS): Int =
        (rate.toLong() * ms / 1000).toInt() * channels * 2

    /** Stop when none of the chosen computers has listened for [idleMs],
     *  once the start's grace ([graceUntil]) is over. */
    fun shouldStop(now: Long, graceUntil: Long, lastListened: Long, idleMs: Long): Boolean =
        now > graceUntil && now - lastListened > idleMs
}
