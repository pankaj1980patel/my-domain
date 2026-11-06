package com.mydomain.android.core

import java.nio.ByteBuffer
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale

/**
 * The arithmetic of the phone's camera on a computer ([com.mydomain.android.CameraShare]),
 * kept apart from Android so it's tested on the JVM: which preview size,
 * which way up, the camera's YUV planes as NV21, turning NV21 a quarter at
 * a time, and the photo's file name.
 */
object CameraFrames {
    /** The longest side of a picture sent while the camera shows. */
    const val MAX_LONG = 1280

    /**
     * The preview size to stream: the largest of [sizes] (width to height)
     * whose long side is at most [maxLong]; with none that small, the
     * smallest there is. Null for an empty list.
     */
    fun pickSize(sizes: List<Pair<Int, Int>>, maxLong: Int = MAX_LONG): Pair<Int, Int>? {
        val ok = sizes.filter { (w, h) -> w > 0 && h > 0 }
        val fits = ok.filter { (w, h) -> maxOf(w, h) <= maxLong }
        return if (fits.isNotEmpty()) {
            fits.maxWithOrNull(compareBy<Pair<Int, Int>> { it.first.toLong() * it.second }.thenBy { it.first })
        } else {
            ok.minByOrNull { it.first.toLong() * it.second }
        }
    }

    /**
     * How the phone is held, to the nearest quarter turn clockwise (0, 90,
     * 180, 270), from an orientation sensor reading in degrees ([raw]; -1
     * when it can't tell, e.g. lying flat on a desk: then [last] stays).
     * A reading near a boundary keeps [last] until it's clearly past it, so
     * a picture doesn't flip back and forth.
     */
    fun quarterTurn(raw: Int, last: Int): Int {
        if (raw < 0) return last
        val r = raw % 360
        val nearest = ((r + 45) / 90 * 90) % 360
        if (nearest == last) return last
        // Within 15 degrees of the 45-degree line between the two: not yet.
        val dist = minOf((r - last + 360) % 360, (last - r + 360) % 360)
        return if (dist < 60) last else nearest
    }

    /**
     * How far to turn the camera's picture clockwise so it's upright: the
     * sensor's mounting ([sensorDeg], Camera2's SENSOR_ORIENTATION) and how
     * the phone is held ([deviceDeg], clockwise). The front camera turns the
     * other way (its picture isn't mirrored here: it's what the camera sees).
     */
    fun rotation(sensorDeg: Int, deviceDeg: Int, front: Boolean): Int {
        val s = ((sensorDeg % 360) + 360) % 360
        val d = ((deviceDeg % 360) + 360) % 360
        return if (front) (s - d + 360) % 360 else (s + d) % 360
    }

    /** A [w] by [h] picture's size once turned [deg] clockwise. */
    fun rotatedSize(w: Int, h: Int, deg: Int): Pair<Int, Int> =
        if (deg == 90 || deg == 270) h to w else w to h

    /**
     * The camera's YUV_420_888 planes as one NV21 buffer (all of Y, then V
     * and U interleaved at half size), whatever the planes' strides.
     * [w] and [h] are even.
     */
    fun toNv21(
        w: Int, h: Int,
        y: ByteBuffer, yRowStride: Int,
        u: ByteBuffer, v: ByteBuffer, uvRowStride: Int, uvPixelStride: Int,
    ): ByteArray {
        val out = ByteArray(w * h * 3 / 2)
        var o = 0
        for (row in 0 until h) {
            val base = row * yRowStride
            for (col in 0 until w) out[o++] = y.get(base + col)
        }
        for (row in 0 until h / 2) {
            val base = row * uvRowStride
            for (col in 0 until w / 2) {
                val i = base + col * uvPixelStride
                out[o++] = v.get(i)
                out[o++] = u.get(i)
            }
        }
        return out
    }

    /**
     * An NV21 picture [w] by [h] turned [deg] clockwise (0, 90, 180 or
     * 270); its size is then [rotatedSize]. [w] and [h] are even.
     */
    fun rotateNv21(src: ByteArray, w: Int, h: Int, deg: Int): ByteArray {
        if (deg % 360 == 0) return src
        val out = ByteArray(src.size)
        val frame = w * h
        val uvW = w / 2
        val uvH = h / 2
        when (deg) {
            90 -> {
                for (y in 0 until h) for (x in 0 until w) out[x * h + (h - 1 - y)] = src[y * w + x]
                for (uy in 0 until uvH) for (ux in 0 until uvW) {
                    val s = frame + uy * w + 2 * ux
                    val d = frame + ux * h + 2 * (uvH - 1 - uy)
                    out[d] = src[s]
                    out[d + 1] = src[s + 1]
                }
            }
            180 -> {
                for (y in 0 until h) for (x in 0 until w) out[(h - 1 - y) * w + (w - 1 - x)] = src[y * w + x]
                for (uy in 0 until uvH) for (ux in 0 until uvW) {
                    val s = frame + uy * w + 2 * ux
                    val d = frame + (uvH - 1 - uy) * w + 2 * (uvW - 1 - ux)
                    out[d] = src[s]
                    out[d + 1] = src[s + 1]
                }
            }
            270 -> {
                for (y in 0 until h) for (x in 0 until w) out[(w - 1 - x) * h + y] = src[y * w + x]
                for (uy in 0 until uvH) for (ux in 0 until uvW) {
                    val s = frame + uy * w + 2 * ux
                    val d = frame + (uvW - 1 - ux) * h + 2 * uy
                    out[d] = src[s]
                    out[d + 1] = src[s + 1]
                }
            }
            else -> throw IllegalArgumentException("not a quarter turn: $deg")
        }
        return out
    }

    /** The photo's file name: "camera-20261106-143005.jpg" (the computer
     *  knows a photo it asked for by the "camera-" start). */
    fun photoName(epochMs: Long): String =
        "camera-" + SimpleDateFormat("yyyyMMdd-HHmmss", Locale.US).format(Date(epochMs)) + ".jpg"
}
