package com.mydomain.android.core

/**
 * Where a press on the computer's picture of this screen lands on the
 * screen. The picture is a fixed-size frame ([frameW] x [frameH]) that
 * mirrors the screen ([screenW] x [screenH] now) scaled to fit and
 * centred, so a turned phone shows with bars on two sides. A spot comes in
 * ten-thousandths of the frame ([SCALE]); it maps to screen pixels, or to
 * null on a bar (nothing of the screen there).
 */
object ScreenTouch {
    const val SCALE = 10_000f

    fun toScreen(x: Int, y: Int, frameW: Int, frameH: Int, screenW: Int, screenH: Int): Pair<Float, Float>? {
        if (frameW <= 0 || frameH <= 0 || screenW <= 0 || screenH <= 0) return null
        val scale = minOf(frameW.toFloat() / screenW, frameH.toFloat() / screenH)
        val padX = (frameW - screenW * scale) / 2f
        val padY = (frameH - screenH * scale) / 2f
        val fx = x.coerceIn(0, SCALE.toInt()) / SCALE * frameW
        val fy = y.coerceIn(0, SCALE.toInt()) / SCALE * frameH
        val sx = (fx - padX) / scale
        val sy = (fy - padY) / scale
        // Half a pixel of slack for rounding at the edges.
        if (sx < -0.5f || sy < -0.5f || sx > screenW + 0.5f || sy > screenH + 0.5f) return null
        return sx.coerceIn(0f, screenW - 1f) to sy.coerceIn(0f, screenH - 1f)
    }
}
