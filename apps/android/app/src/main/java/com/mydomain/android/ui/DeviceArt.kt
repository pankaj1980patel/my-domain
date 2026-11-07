package com.mydomain.android.ui

import androidx.compose.foundation.Canvas
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.CornerRadius
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.geometry.RoundRect
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.Path
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.graphics.drawscope.DrawScope
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.graphics.drawscope.clipPath
import androidx.compose.ui.graphics.drawscope.rotate
import androidx.compose.ui.graphics.drawscope.scale
import androidx.compose.ui.graphics.drawscope.translate
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.role
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.unit.IntSize
import androidx.compose.ui.unit.dp
import com.mydomain.android.core.DeviceKinds
import com.mydomain.android.core.Wallpapers
import kotlin.math.abs
import kotlin.math.cos
import kotlin.math.max
import kotlin.math.roundToInt
import kotlin.math.sin

// A device drawn as itself: a hand-made frame (phone, tablet, laptop or
// monitor, from its device icon) with its wallpaper on the screen. The same
// drawing as the desktop's DeviceArt.tsx. On the screen, in order:
//  1. the picture it shared (cover);
//  2. a gradient of the colours it shared;
//  3. a built-in wallpaper: original abstract art in the app's deep-space
//     colours (flowing ribbons and a glow), one of five, always the same one
//     for the same device.
// No line icon as the art, no images or brand artwork: all drawn here.

enum class Frame { Phone, Tablet, Laptop, Monitor }

/** The frame for a device icon name (`features::device_kind::ICONS`). */
fun frameOf(icon: String): Frame = when {
    Regex("phone|foldable|watch").containsMatchIn(icon) -> Frame.Phone
    "tablet" in icon -> Frame.Tablet
    "laptop" in icon -> Frame.Laptop
    icon == "unknown" -> Frame.Phone
    else -> Frame.Monitor
}

/** The art's width for a [height], by frame. */
fun artWidth(frame: Frame, height: Dp): Dp = when (frame) {
    Frame.Phone -> height * 0.49f
    Frame.Tablet -> height * 0.76f
    Frame.Laptop -> height * 0.88f * 1.55f * 1.16f
    Frame.Monitor -> height * 0.74f * 1.6f
}

/** A built-in wallpaper: a night sky from [top] to [bottom], three ribbons
 *  of light and a [glow], turned by [turn] degrees. */
private class Art(val top: Long, val bottom: Long, val ribbons: List<Long>, val glow: Long, val turn: Float)

private val ARTS = listOf(
    // Blue waves.
    Art(0xFF040A26, 0xFF0D1D55, listOf(0xFF2F6BFF, 0xFF7A4DFF, 0xFF3FC8FF), 0xFF3D5BFF, -28f),
    // Violet aurora.
    Art(0xFF08051D, 0xFF200B4A, listOf(0xFF9B5CFF, 0xFFFF5FA8, 0xFF4F7DFF), 0xFF8A3DFF, 22f),
    // Teal aurora.
    Art(0xFF02121A, 0xFF06283D, listOf(0xFF19D3C5, 0xFF3A7BFF, 0xFF7CF2B0), 0xFF12A7C9, -12f),
    // Indigo drift.
    Art(0xFF060A1C, 0xFF161B44, listOf(0xFF5E6BFF, 0xFFB06BFF, 0xFF2BD0FF), 0xFF5B4DFF, 36f),
    // Dusk orbit.
    Art(0xFF0B0716, 0xFF2A0F33, listOf(0xFFFF7A59, 0xFFB84DFF, 0xFFFFB36B), 0xFFC2477A, -40f),
)

/** The same built-in wallpaper for the same device (FNV-1a, as the desktop). */
fun artIndex(key: String): Int {
    var h = 2166136261L.toInt()
    for (c in key) h = (h xor c.code) * 16777619
    return (h.toUInt() % ARTS.size.toUInt()).toInt()
}

/** A ribbon across the 100-unit box: its top edge a wave from [y]. */
private fun ribbon(y: Float, amp: Float, thick: Float): Path = Path().apply {
    moveTo(-20f, y)
    cubicTo(20f, y - amp, 60f, y + amp, 120f, y - amp / 2)
    lineTo(120f, y - amp / 2 + thick)
    cubicTo(60f, y + amp + thick * 1.6f, 20f, y - amp + thick * 0.4f, -20f, y + thick)
    close()
}

/** A few faint stars, the same for the same seed (the desktop's). */
private fun stars(seed: Int): List<Triple<Float, Float, Float>> {
    var s = seed.toLong() * 9301 + 49297
    fun next(): Float {
        s = (s * 9301 + 49297) % 233280
        return s / 233280f
    }
    return List(14) { Triple(next() * 100f, next() * 55f, 0.25f + next() * 0.45f) }
}

/** The built-in art, covering [size] (centred, like `slice`). */
private fun DrawScope.drawDefaultArt(index: Int, box: Size) {
    val art = ARTS.getOrElse(index) { ARTS[0] }
    val k = max(box.width, box.height) / 100f
    translate((box.width - 100f * k) / 2f, (box.height - 100f * k) / 2f) {
        scale(k, k, pivot = Offset.Zero) {
            drawRect(Brush.verticalGradient(listOf(Color(art.top), Color(art.bottom)), 0f, 100f), Offset.Zero, Size(100f, 100f))
            stars(index + 1).forEach { (x, y, r) -> drawCircle(Color.White.copy(alpha = 0.55f), r, Offset(x, y)) }
            rotate(art.turn, pivot = Offset(50f, 50f)) {
                // The glow: an ellipse 120 x 68 with a soft centre.
                scale(1f, 34f / 60f, pivot = Offset(50f, 58f)) {
                    drawCircle(
                        Brush.radialGradient(listOf(Color(art.glow).copy(alpha = 0.75f), Color(art.glow).copy(alpha = 0f)), Offset(50f, 58f), 66f),
                        60f, Offset(50f, 58f),
                    )
                }
                val shapes = listOf(ribbon(40f, 16f, 12f) to 1f, ribbon(55f, 22f, 8f) to 0.85f, ribbon(64f, 12f, 5f) to 0.9f)
                shapes.forEachIndexed { i, (p, alpha) ->
                    val c = Color(art.ribbons[i])
                    drawPath(
                        p,
                        Brush.horizontalGradient(
                            0f to c.copy(alpha = 0f), 0.45f to c.copy(alpha = 0.9f), 1f to c.copy(alpha = 0.15f),
                            startX = -20f, endX = 120f,
                        ),
                        alpha = alpha,
                    )
                }
                drawPath(ribbon(50f, 18f, 1.2f), Color.White.copy(alpha = 0.35f))
            }
        }
    }
}

/** A gradient from the shared colours: the main one across, the others as
 *  soft glows from two corners (the desktop's gradientOf). */
private fun DrawScope.drawColors(colors: List<Int>, box: Size) {
    val a = Color(colors[0])
    val b = Color(colors.getOrElse(1) { colors[0] })
    val d = Color(colors.getOrElse(2) { colors.getOrElse(1) { colors[0] } })
    // linear-gradient(150deg, a, b 55%, d)
    val rad = Math.toRadians(150.0)
    val dx = sin(rad).toFloat()
    val dy = -cos(rad).toFloat()
    val half = (abs(box.width * dx) + abs(box.height * dy)) / 2f
    val c = Offset(box.width / 2f, box.height / 2f)
    drawRect(Brush.linearGradient(0f to a, 0.55f to b, 1f to d, start = c - Offset(dx, dy) * half, end = c + Offset(dx, dy) * half), size = box)
    val m = max(box.width, box.height)
    drawRect(Brush.radialGradient(listOf(d, d.copy(alpha = 0f)), Offset(box.width * 0.9f, box.height * 0.95f), m * 0.6f), size = box)
    drawRect(Brush.radialGradient(listOf(b, b.copy(alpha = 0f)), Offset(box.width * 0.15f, box.height * 0.1f), m * 0.62f), size = box)
}

private val BODY = listOf(Color(0xFF3A3F4C), Color(0xFF151820), Color(0xFF0A0C12))
private val METAL = listOf(Color(0xFFD9DDE5), Color(0xFFA5ABB8), Color(0xFF6E7482))

private fun bodyBrush(r: Rect): Brush {
    val rad = Math.toRadians(150.0)
    val v = Offset(sin(rad).toFloat(), -cos(rad).toFloat())
    val half = (abs(r.width * v.x) + abs(r.height * v.y)) / 2f
    return Brush.linearGradient(0f to BODY[0], 0.45f to BODY[1], 1f to BODY[2], start = r.center - v * half, end = r.center + v * half)
}

/**
 * [node]'s device drawn with its wallpaper. [icon]: its device icon name
 * (picks the frame; default: what it is, or the one you chose). [height]:
 * the art's height (its width follows the frame), made smaller when the
 * width would pass [maxWidth]. [title]: read out for the picture (its
 * name); none: decorative (the name is next to it).
 */
@Composable
fun DeviceArt(
    node: String,
    height: Dp,
    modifier: Modifier = Modifier,
    maxWidth: Dp = Dp.Unspecified,
    icon: String = DeviceKinds.iconOf(node),
    title: String? = null,
) {
    val frame = frameOf(icon)
    val ratio = artWidth(frame, 100.dp) / 100.dp
    val h = if (maxWidth != Dp.Unspecified && height * ratio > maxWidth) maxWidth / ratio else height
    val look = Wallpapers.looks[node]
    val image: ImageBitmap? = remember(look) { look?.image?.asImageBitmap() }
    val colors = look?.colors.orEmpty()
    val art = remember(node) { artIndex(node) }
    val sem = if (title != null) {
        Modifier.clearAndSetSemantics { contentDescription = title; role = Role.Image }
    } else {
        Modifier.clearAndSetSemantics {}
    }
    Canvas(modifier.size(artWidth(frame, h), h).then(sem)) {
        val H = size.height
        val W = size.width

        /** What the screen shows, in [r] with corner [radius]. */
        fun screen(r: Rect, radius: Float) {
            val clip = Path().apply { addRoundRect(RoundRect(r, CornerRadius(radius))) }
            clipPath(clip) {
                translate(r.left, r.top) {
                    when {
                        image != null -> {
                            drawRect(Color(0xFF05081A), size = r.size)
                            // Cover: fill the screen, cropping the overflow.
                            val k = max(r.width / image.width, r.height / image.height)
                            val w = image.width * k
                            val h = image.height * k
                            drawImage(
                                image,
                                dstOffset = IntOffset(((r.width - w) / 2f).roundToInt(), ((r.height - h) / 2f).roundToInt()),
                                dstSize = IntSize(w.roundToInt(), h.roundToInt()),
                            )
                        }
                        colors.isNotEmpty() -> drawColors(colors, r.size)
                        else -> drawDefaultArt(art, r.size)
                    }
                    // Glass: a faint sheen across the screen.
                    drawRect(
                        Brush.linearGradient(
                            0f to Color.White.copy(alpha = 0.14f), 0.38f to Color.White.copy(alpha = 0f),
                            start = Offset.Zero, end = Offset(r.width, r.height),
                        ),
                        size = r.size,
                    )
                }
            }
        }

        fun body(r: Rect, radius: CornerRadius, bottom: CornerRadius = radius) {
            val rr = RoundRect(r, radius, radius, bottom, bottom)
            val p = Path().apply { addRoundRect(rr) }
            drawPath(p, bodyBrush(r))
            drawPath(p, Color.White.copy(alpha = 0.16f), style = Stroke(1.dp.toPx()))
        }

        when (frame) {
            Frame.Phone, Frame.Tablet -> {
                val phone = frame == Frame.Phone
                val bezel = max(2f, H * if (phone) 0.028f else 0.045f)
                val r = H * if (phone) 0.11f else 0.06f
                val hole = max(2f, H * 0.03f)
                body(Rect(0f, 0f, W, H), CornerRadius(r))
                screen(Rect(bezel, bezel, W - bezel, H - bezel), max(1f, r - bezel))
                // The camera: a punch-hole on a phone, a dot in the bezel on a tablet.
                val camR = (if (phone) hole else hole * 0.6f) / 2f
                val camY = if (phone) bezel + hole * 0.9f + camR else bezel / 2f
                drawCircle(Color(0xFF05060A), camR, Offset(W / 2f, camY))
                drawCircle(Color.White.copy(alpha = 0.12f), camR, Offset(W / 2f, camY), style = Stroke(1f))
            }
            Frame.Laptop -> {
                val lidH = H * 0.88f
                val lidW = lidH * 1.55f
                val baseH = max(3f, H * 0.07f)
                val baseW = W
                val bezel = max(2f, H * 0.035f)
                val r = max(2f, H * 0.045f)
                val lid = Rect((baseW - lidW) / 2f, 0f, (baseW + lidW) / 2f, lidH)
                body(lid, CornerRadius(r), CornerRadius(2f))
                screen(Rect(lid.left + bezel, bezel, lid.right - bezel, lidH - bezel * 1.2f), max(1f, r - bezel))
                // The base, with the notch to open it.
                val base = Rect(0f, lidH, baseW, lidH + baseH)
                drawPath(
                    Path().apply { addRoundRect(RoundRect(base, CornerRadius(1f), CornerRadius(1f), CornerRadius(baseH * 0.9f), CornerRadius(baseH * 0.9f))) },
                    Brush.verticalGradient(0f to METAL[0], 0.55f to METAL[1], 1f to METAL[2], startY = base.top, endY = base.bottom),
                )
                val notchW = baseW * 0.14f
                val notchH = max(1f, baseH * 0.35f)
                drawRoundRect(
                    Color.Black.copy(alpha = 0.22f), Offset((baseW - notchW) / 2f, lidH), Size(notchW, notchH),
                    CornerRadius(notchH),
                )
            }
            Frame.Monitor -> {
                val scrH = H * 0.74f
                val scrW = W
                val footH = max(2f, H * 0.07f)
                val bezel = max(2f, H * 0.03f)
                val r = max(2f, H * 0.035f)
                val metal = Brush.verticalGradient(0f to METAL[0], 0.55f to METAL[1], 1f to METAL[2], startY = scrH, endY = H)
                // The stand: a neck that widens down to a foot.
                drawPath(
                    Path().apply {
                        val l = scrW * 0.44f
                        val w = scrW * 0.12f
                        moveTo(l + w * 0.12f, scrH - 2f)
                        lineTo(l + w * 0.88f, scrH - 2f)
                        lineTo(l + w, H - footH + 1f)
                        lineTo(l, H - footH + 1f)
                        close()
                    },
                    metal,
                )
                drawPath(
                    Path().apply {
                        addRoundRect(
                            RoundRect(
                                Rect(scrW * 0.32f, H - footH, scrW * 0.68f, H),
                                CornerRadius(footH), CornerRadius(footH), CornerRadius(2f), CornerRadius(2f),
                            ),
                        )
                    },
                    metal,
                )
                body(Rect(0f, 0f, scrW, scrH), CornerRadius(r))
                screen(Rect(bezel, bezel, scrW - bezel, scrH - bezel), max(1f, r - bezel))
            }
        }
    }
}
