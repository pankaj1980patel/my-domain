package com.mydomain.android.core

/**
 * IMA ADPCM (4 bits a sample) and RED-style redundancy, exactly as the
 * core codes audio records (mdcore `features::audio_codec`), checked
 * against the same test vectors. The core decodes the records it receives
 * before they reach the app; this is the same coder on the JVM side, for
 * code that handles the bytes itself, and as a check that both agree.
 *
 * Payload: `[u16 samples per channel][block per channel]`, a block
 * `[i16 predictor][u8 step index][u8 0][nibbles, low first]`; RED:
 * `[u16 primary len][primary][u8 seq back][redundant]`. Little-endian.
 */
object ImaAdpcm {
    private val STEPS = intArrayOf(
        7, 8, 9, 10, 11, 12, 13, 14, 16, 17, 19, 21, 23, 25, 28, 31, 34, 37, 41, 45, 50, 55, 60, 66,
        73, 80, 88, 97, 107, 118, 130, 143, 157, 173, 190, 209, 230, 253, 279, 307, 337, 371, 408, 449,
        494, 544, 598, 658, 724, 796, 876, 963, 1060, 1166, 1282, 1411, 1552, 1707, 1878, 2066, 2272,
        2499, 2749, 3024, 3327, 3660, 4026, 4428, 4871, 5358, 5894, 6484, 7132, 7845, 8630, 9493,
        10442, 11487, 12635, 13899, 15289, 16818, 18500, 20350, 22385, 24623, 27086, 29794, 32767,
    )
    private val INDEX_STEP = intArrayOf(-1, -1, -1, -1, 2, 4, 6, 8, -1, -1, -1, -1, 2, 4, 6, 8)

    /** One channel's coder state. */
    class State(var predictor: Int = 0, var index: Int = 0) {
        fun step(nibble: Int): Short {
            val step = STEPS[index]
            var delta = step shr 3
            if (nibble and 4 != 0) delta += step
            if (nibble and 2 != 0) delta += step shr 1
            if (nibble and 1 != 0) delta += step shr 2
            predictor = (if (nibble and 8 != 0) predictor - delta else predictor + delta).coerceIn(-32768, 32767)
            index = (index + INDEX_STEP[nibble and 15]).coerceIn(0, 88)
            return predictor.toShort()
        }

        fun encode(sample: Short): Int {
            val step = STEPS[index]
            var diff = sample - predictor
            var nibble = 0
            if (diff < 0) {
                nibble = 8
                diff = -diff
            }
            if (diff >= step) {
                nibble = nibble or 4
                diff -= step
            }
            if (diff >= step shr 1) {
                nibble = nibble or 2
                diff -= step shr 1
            }
            if (diff >= step shr 2) nibble = nibble or 1
            step(nibble)
            return nibble
        }
    }

    /** The sender's coder: one state per channel, running on across frames. */
    class Encoder {
        private var states = emptyArray<State>()

        /** Code [pcm] (s16le, [channels] interleaved) as one payload. */
        fun encode(pcm: ByteArray, channels: Int): ByteArray {
            val ch = maxOf(channels, 1)
            if (states.size != ch) states = Array(ch) { State() }
            val n = minOf(pcm.size / 2 / ch, 0xffff)
            val block = 4 + (n + 1) / 2
            val out = ByteArray(2 + ch * block)
            out[0] = n.toByte()
            out[1] = (n shr 8).toByte()
            for (c in 0 until ch) {
                val st = states[c]
                var w = 2 + c * block
                out[w++] = st.predictor.toByte()
                out[w++] = (st.predictor shr 8).toByte()
                out[w++] = st.index.toByte()
                out[w++] = 0
                for (i in 0 until n) {
                    val at = 2 * (i * ch + c)
                    val s = ((pcm[at].toInt() and 0xff) or (pcm[at + 1].toInt() shl 8)).toShort()
                    val nib = st.encode(s)
                    if (i % 2 == 0) {
                        out[w] = nib.toByte()
                    } else {
                        out[w] = (out[w].toInt() or (nib shl 4)).toByte()
                        w++
                    }
                }
            }
            return out
        }
    }

    /** Bytes a payload of [samples] per channel takes. */
    fun encodedLen(samples: Int, channels: Int): Int = 2 + maxOf(channels, 1) * (4 + (samples + 1) / 2)

    /** Decode a payload into s16le PCM; null when it's damaged. */
    fun decode(data: ByteArray, channels: Int): ByteArray? {
        if (channels <= 0 || data.size < 2) return null
        val n = (data[0].toInt() and 0xff) or ((data[1].toInt() and 0xff) shl 8)
        val block = 4 + (n + 1) / 2
        if (data.size != 2 + channels * block) return null
        val out = ByteArray(n * channels * 2)
        for (c in 0 until channels) {
            val b = 2 + c * block
            val st = State(
                predictor = ((data[b].toInt() and 0xff) or (data[b + 1].toInt() shl 8)).toShort().toInt(),
                index = data[b + 2].toInt() and 0xff,
            )
            if (st.index > 88) return null
            for (i in 0 until n) {
                val byte = data[b + 4 + i / 2].toInt() and 0xff
                val nib = if (i % 2 == 0) byte and 15 else byte shr 4
                val s = st.step(nib).toInt()
                val at = 2 * (i * channels + c)
                out[at] = s.toByte()
                out[at + 1] = (s shr 8).toByte()
            }
        }
        return out
    }

    /** A record's payload and the one [back] records before it, together. */
    fun packRed(primary: ByteArray, back: Int, redundant: ByteArray): ByteArray {
        val len = minOf(primary.size, 0xffff)
        val out = ByteArray(3 + len + redundant.size)
        out[0] = len.toByte()
        out[1] = (len shr 8).toByte()
        System.arraycopy(primary, 0, out, 2, len)
        out[2 + len] = back.toByte()
        System.arraycopy(redundant, 0, out, 3 + len, redundant.size)
        return out
    }

    /** A RED payload split: primary, how many records back, redundant. */
    class Red(val primary: ByteArray, val back: Int, val redundant: ByteArray)

    fun unpackRed(data: ByteArray): Red? {
        if (data.size < 2) return null
        val len = (data[0].toInt() and 0xff) or ((data[1].toInt() and 0xff) shl 8)
        if (data.size < 3 + len) return null
        val back = data[2 + len].toInt() and 0xff
        if (back == 0) return null
        return Red(data.copyOfRange(2, 2 + len), back, data.copyOfRange(3 + len, data.size))
    }
}
