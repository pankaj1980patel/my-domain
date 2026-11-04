package com.mydomain.android.core

/**
 * Audio waiting to be played: a computer's sound on this phone
 * (ComputerAudio.kt), kept free of Android so it's tested on the JVM. The
 * same rules as the desktop's buffer for a phone's audio (phone_audio.rs
 * `Jitter`): playing starts once [targetMs] is buffered; a frame that comes
 * after a later one is dropped; more than [maxMs] piled up (a burst after
 * a stall, or the computer's clock running ahead) is trimmed back to
 * [targetMs]; running dry counts an underrun and waits for [targetMs]
 * again. A new rate or channel count starts over.
 *
 * Holds stereo 16-bit samples at the computer's rate (mono is doubled);
 * the player opens its output at [rate]. Thread-safe: frames arrive on the
 * core's thread, the player pulls on its own.
 */
class AudioJitter(val targetMs: Int = TARGET_MS, maxMs: Int = MAX_MS) {
    val maxMs: Int = maxOf(maxMs, targetMs)

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

    /** Times it ran dry while playing. */
    var underruns = 0
        private set
    /** Frames that came after a later one (dropped). */
    var late = 0
        private set
    /** Times it was trimmed for holding too much. */
    var trimmed = 0
        private set

    @Synchronized fun playing(): Boolean = !priming

    private fun framesFor(ms: Int): Int = (rate.toLong() * ms / 1000).toInt()

    /** How much is waiting, in milliseconds. */
    @Synchronized fun bufferedMs(): Int =
        if (rate == 0) 0 else ((len / 2).toLong() * 1000 / rate).toInt()

    /** Start over (another computer: its frames count from their own
     *  start). */
    @Synchronized fun reset() {
        rate = 0
        channels = 0
        start = 0
        len = 0
        nextSeq = null
        priming = true
    }

    /** A frame from the computer: [pcm] is s16le, [channels] interleaved,
     *  at [rate] Hz. */
    @Synchronized fun push(seq: Long, rate: Int, channels: Int, pcm: ByteArray) {
        if (rate <= 0 || channels !in 1..2) return
        if (rate != this.rate || channels != this.channels) {
            this.rate = rate
            this.channels = channels
            start = 0
            len = 0
            nextSeq = null
            priming = true
        }
        val next = nextSeq
        if (next != null && seq < next) {
            late++
            return
        }
        nextSeq = seq + 1
        val samples = pcm.size / 2
        val add = if (channels == 1) samples * 2 else samples - samples % 2
        ensure(len + add)
        var w = start + len
        var i = 0
        while (i + 1 < pcm.size && i / 2 < samples) {
            val s = ((pcm[i].toInt() and 0xff) or (pcm[i + 1].toInt() shl 8)).toShort()
            if (channels == 1) {
                buf[w++] = s
                buf[w++] = s
            } else if (i / 2 < add) {
                buf[w++] = s
            }
            i += 2
        }
        len += add
        val frames = len / 2
        if (frames > framesFor(maxMs)) {
            val excess = frames - framesFor(targetMs)
            start += excess * 2
            len -= excess * 2
            trimmed++
        }
        if (priming && len / 2 >= maxOf(framesFor(targetMs), 1)) priming = false
    }

    /**
     * Fill [out] with up to [frames] stereo frames to play (interleaved).
     * Silence while waiting for [targetMs]; running dry part way counts an
     * underrun, fills the rest with silence and waits again. Always fills
     * [frames] frames.
     */
    @Synchronized fun pull(out: ShortArray, frames: Int) {
        val want = minOf(frames, out.size / 2) * 2
        if (priming) {
            out.fill(0, 0, want)
            return
        }
        val n = minOf(want, len)
        System.arraycopy(buf, start, out, 0, n)
        start += n
        len -= n
        if (n < want) {
            out.fill(0, n, want)
            underruns++
            priming = true
        }
    }

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

    companion object {
        /** Buffered before playing starts (and again after an underrun). */
        const val TARGET_MS = 80
        /** More than this buffered: trimmed back to [TARGET_MS]. */
        const val MAX_MS = 200
    }
}
