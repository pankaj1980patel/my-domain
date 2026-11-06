package com.mydomain.android.core

/**
 * Audio waiting to be played: a computer's sound on this phone
 * (ComputerAudio.kt), kept free of Android so it's tested on the JVM. The
 * same rules, constants and tests as the core's buffer the desktop plays a
 * phone's audio through (mdcore `features::audio_jitter`):
 *
 *  - **Adaptive target.** Playing starts once [targetMs] is buffered.
 *    Running dry grows the target by [GROW_UNDERRUN_MS], a frame that came
 *    too late by [GROW_LATE_MS] (fast), up to [CEIL_MS]; [STABLE_MS] of
 *    playing with neither shrinks it by [SHRINK_MS] (slowly), down to the
 *    floor: [FLOOR_MS], or [FLOOR_NEARBY_MS] on a nearby low-latency link
 *    ([setLowLatency]), which is all nearby mode changes here.
 *  - **Concealment.** Running dry repeats the last [HISTORY_MS] played,
 *    fading out over [CONCEAL_MS]; when audio comes back it crossfades in
 *    over [XFADE_MS] (no click). Only then does it wait for the target.
 *  - **Drift.** The computer's clock and this phone's never quite agree:
 *    instead of dropping or inserting frames, it plays up to [DRIFT_MAX]
 *    (±0.5 %) faster or slower, by linear interpolation, to keep the fill
 *    at the target. Only a pile-up past [CEIL_MS] + target is trimmed.
 *
 * Holds stereo 16-bit samples at the computer's rate (mono is doubled);
 * the player opens its output at [rate], so the only resampling is the
 * drift's. Thread-safe: frames arrive on the core's thread, the player
 * pulls on its own. [pull] allocates nothing.
 */
class AudioJitter {
    /** The computer's rate, Hz (0 until the first frame). */
    var rate = 0
        private set
    private var channels = 0
    /** Interleaved L, R. */
    private var buf = ShortArray(0)
    private var start = 0
    private var len = 0
    private var nextSeq: Long? = null
    private var priming = true
    /** Where the next output frame falls between the first two frames. */
    private var pos = 0.0
    @Volatile var targetMs = START_MS
        private set
    private var floorMs = FLOOR_MS
    private var fillMs = 0.0
    private var ratio = 1.0
    /** Output frames played since the last trouble. */
    private var calm = 0L
    /** The last frames played (L, R), a ring, for concealment. */
    private var history = FloatArray(0)
    private var histCount = 0
    private var histNext = 0
    /** Frames made up so far while concealing; -1: not concealing. */
    private var concealing = -1L
    private var xfadeLeft = 0
    private var xfadeFrom = 0f
    private var concealAt = 0
    private var outL = 0f
    private var outR = 0f

    /** Times it ran dry while playing. */
    var underruns = 0
        private set
    /** Frames that came after a later one (dropped). */
    var late = 0
        private set
    /** Times a pile-up was trimmed. */
    var trimmed = 0
        private set
    private var concealedFrames = 0L

    @Synchronized fun playing(): Boolean = !priming

    @Synchronized fun lowLatency(): Boolean = floorMs == FLOOR_NEARBY_MS

    /** On a nearby low-latency link the target may shrink lower. */
    @Synchronized fun setLowLatency(on: Boolean) {
        floorMs = if (on) FLOOR_NEARBY_MS else FLOOR_MS
        targetMs = maxOf(targetMs, floorMs)
    }

    private fun frames(): Int = len / 2

    private fun framesFor(ms: Int): Int = (rate.toLong() * ms / 1000).toInt()

    /** How much is waiting, in milliseconds. */
    @Synchronized fun bufferedMs(): Int =
        if (rate == 0) 0 else (frames().toLong() * 1000 / rate).toInt()

    /** What the audio view shows. */
    data class Stats(
        val bufferedMs: Int,
        val targetMs: Int,
        val underruns: Int,
        val concealedMs: Int,
        val late: Int,
        val trimmed: Int,
        val driftPpm: Int,
    )

    @Synchronized fun stats(): Stats = Stats(
        bufferedMs = bufferedMs(),
        targetMs = targetMs,
        underruns = underruns,
        concealedMs = if (rate == 0) 0 else (concealedFrames * 1000 / rate).toInt(),
        late = late,
        trimmed = trimmed,
        driftPpm = Math.round((ratio - 1.0) * 1e6).toInt(),
    )

    /** Start over (another computer: its frames count from their own start). */
    @Synchronized fun reset() {
        rate = 0
        channels = 0
        start = 0
        len = 0
        nextSeq = null
        priming = true
        pos = 0.0
        histCount = 0
        histNext = 0
        concealing = -1
        xfadeLeft = 0
    }

    private fun trouble(e: Event) {
        targetMs = adapt(targetMs, floorMs, e)
        calm = 0
    }

    /** A frame from the computer: [pcm] is s16le, [channels] interleaved,
     *  at [rate] Hz. A new format starts over; a late frame is dropped (and
     *  grows the target). */
    @Synchronized fun push(seq: Long, rate: Int, channels: Int, pcm: ByteArray) {
        if (rate <= 0 || channels !in 1..2) return
        if (rate != this.rate || channels != this.channels) {
            this.rate = rate
            this.channels = channels
            start = 0
            len = 0
            nextSeq = null
            priming = true
            pos = 0.0
            history = FloatArray(maxOf(rate * HISTORY_MS / 1000, 1) * 2)
            histCount = 0
            histNext = 0
        }
        val next = nextSeq
        if (next != null && seq < next) {
            late++
            trouble(Event.Late)
            return
        }
        nextSeq = seq + 1
        val samples = pcm.size / 2
        val add = if (channels == 1) samples * 2 else samples - samples % 2
        ensure(len + add)
        var w = start + len
        var i = 0
        while (i < samples) {
            val s = ((pcm[2 * i].toInt() and 0xff) or (pcm[2 * i + 1].toInt() shl 8)).toShort()
            if (channels == 1) {
                buf[w++] = s
                buf[w++] = s
            } else if (i < add) {
                buf[w++] = s
            }
            i++
        }
        len += add
        // Far past the ceiling (a burst after a stall): back to the target.
        if (frames() > framesFor(CEIL_MS + targetMs)) {
            val excess = frames() - framesFor(targetMs)
            start += excess * 2
            len -= excess * 2
            pos = 0.0
            trimmed++
        }
    }

    /**
     * Fill [out] with [frames] stereo frames to play (interleaved), at the
     * stream's rate nudged for drift. Silence while waiting for the target;
     * running dry conceals, then waits again. Always fills [frames] frames.
     */
    @Synchronized fun pull(out: ShortArray, frames: Int) {
        val n = minOf(frames, out.size / 2)
        if (rate == 0) {
            out.fill(0, 0, n * 2)
            return
        }
        val nowMs = frames().toDouble() * 1000.0 / rate
        fillMs += (nowMs - fillMs) * FILL_SMOOTH
        ratio = if (priming) 1.0 else driftRatio(fillMs, targetMs.toDouble())
        val concealMax = rate.toLong() * CONCEAL_MS / 1000
        val xfadeFrames = maxOf(rate * XFADE_MS / 1000, 1)
        val stable = rate.toLong() * STABLE_MS / 1000
        for (k in 0 until n) {
            next(ratio, concealMax, xfadeFrames)
            out[2 * k] = toShort(outL)
            out[2 * k + 1] = toShort(outR)
            if (!priming && concealing < 0) {
                calm++
                if (calm >= stable) trouble(Event.Stable)
            }
        }
    }

    /** One output frame, into [outL] / [outR]. */
    private fun next(step: Double, concealMax: Long, xfadeFrames: Int) {
        if (priming) {
            if (frames() >= maxOf(framesFor(targetMs), 2)) {
                priming = false
                // Fade in from silence.
                xfadeLeft = xfadeFrames
                xfadeFrom = 0f
            } else {
                outL = 0f
                outR = 0f
                return
            }
        }
        val i = pos.toInt()
        if (i + 1 < frames()) {
            if (concealing >= 0) {
                xfadeFrom = concealGain(concealMax)
                xfadeLeft = xfadeFrames
                concealing = -1
            }
            val f = (pos - i).toFloat()
            val a = start + 2 * i
            var l = buf[a] + (buf[a + 2] - buf[a]) * f
            var r = buf[a + 1] + (buf[a + 3] - buf[a + 1]) * f
            if (xfadeLeft > 0) {
                val t = 1f - xfadeLeft.toFloat() / xfadeFrames
                repeat()
                l = l * t + outL * xfadeFrom * (1f - t)
                r = r * t + outR * xfadeFrom * (1f - t)
                xfadeLeft--
            }
            pos += step
            val whole = pos.toInt()
            if (whole > 0) {
                val drop = minOf(whole * 2, len)
                start += drop
                len -= drop
                pos -= whole
            }
            remember(l, r)
            outL = l
            outR = r
            return
        }
        // Dry.
        val c = if (concealing >= 0) concealing else {
            underruns++
            trouble(Event.Underrun)
            concealAt = 0
            0L
        }
        if (c < concealMax && histCount > 0) {
            concealing = c + 1
            concealedFrames++
            val g = concealGain(concealMax)
            repeat()
            outL *= g
            outR *= g
            return
        }
        concealing = -1
        xfadeLeft = 0
        priming = true
        outL = 0f
        outR = 0f
    }

    private fun concealGain(concealMax: Long): Float {
        val c = if (concealing < 0) 0L else concealing
        return 1f - minOf(c.toFloat() / maxOf(concealMax, 1L), 1f)
    }

    private fun remember(l: Float, r: Float) {
        if (history.isEmpty()) return
        val max = history.size / 2
        history[2 * histNext] = l
        history[2 * histNext + 1] = r
        histNext = (histNext + 1) % max
        if (histCount < max) histCount++
    }

    /** The next frame of the repeated history, into [outL] / [outR]. */
    private fun repeat() {
        if (histCount == 0) {
            outL = 0f
            outR = 0f
            return
        }
        val max = history.size / 2
        val oldest = if (histCount < max) 0 else histNext
        val k = (oldest + concealAt % histCount) % max
        concealAt++
        outL = history[2 * k]
        outR = history[2 * k + 1]
    }

    private fun toShort(v: Float): Short = v.coerceIn(-32768f, 32767f).toInt().toShort()

    /** Room for [total] samples after [start]: compact, or grow. */
    private fun ensure(total: Int) {
        if (start + total <= buf.size) return
        if (total <= buf.size) {
            System.arraycopy(buf, start, buf, 0, len)
        } else {
            val bigger = ShortArray(maxOf(total, buf.size * 2))
            System.arraycopy(buf, start, bigger, 0, len)
            buf = bigger
        }
        start = 0
    }

    /** What moves the target. */
    enum class Event { Underrun, Late, Stable }

    companion object {
        const val START_MS = 80
        const val FLOOR_MS = 60
        const val FLOOR_NEARBY_MS = 30
        const val CEIL_MS = 200
        const val GROW_UNDERRUN_MS = 20
        const val GROW_LATE_MS = 10
        const val STABLE_MS = 5_000
        const val SHRINK_MS = 10
        const val CONCEAL_MS = 60
        const val HISTORY_MS = 10
        const val XFADE_MS = 5
        const val DRIFT_MAX = 0.005
        const val DRIFT_GAIN = 0.01
        const val FILL_SMOOTH = 0.02

        /** The adaptive target after [e]. */
        fun adapt(targetMs: Int, floorMs: Int, e: Event): Int = maxOf(
            floorMs,
            when (e) {
                Event.Underrun -> minOf(targetMs + GROW_UNDERRUN_MS, CEIL_MS)
                Event.Late -> minOf(targetMs + GROW_LATE_MS, CEIL_MS)
                Event.Stable -> maxOf(targetMs - SHRINK_MS, floorMs)
            },
        )

        /** The playback speed that steers a buffer at [fillMs] towards
         *  [targetMs]: above 1 plays faster (drains). */
        fun driftRatio(fillMs: Double, targetMs: Double): Double {
            if (targetMs <= 0.0) return 1.0
            val err = (fillMs - targetMs) / targetMs
            return 1.0 + (err * DRIFT_GAIN).coerceIn(-DRIFT_MAX, DRIFT_MAX)
        }
    }
}
