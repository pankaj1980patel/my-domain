package com.mydomain.android.core

import kotlin.math.abs
import kotlin.math.hypot
import kotlin.math.max
import kotlin.math.min
import kotlin.math.sqrt

/**
 * The arithmetic behind "Scan documents with <phone>" (InsertCapture.kt),
 * pure Kotlin on plain arrays so it runs in JVM tests:
 *
 *  - finding the page in a photo: grayscale, scaled down to about 256 px
 *    (averaging smooths the noise), Sobel edges above a threshold, the
 *    connected edge outlines, and of those the largest convex quadrilateral
 *    that fills its outline well (a page's border), corners in the order
 *    top left, top right, bottom right, bottom left. None found: the whole
 *    picture, a little inset, for you to drag into place;
 *  - the perspective (a homography from four corners) and how big the
 *    straightened page comes out;
 *  - black and white: an adaptive threshold (each pixel against the mean of
 *    its neighbourhood, from an integral image), so shadows and uneven light
 *    don't swallow the text;
 *  - the pages (move, delete, keep order) and fitting each on A4 or Letter.
 */

/** A point, in pixels of the picture it's about. */
data class Pt(val x: Double, val y: Double)

/** A page's four corners: top left, top right, bottom right, bottom left. */
data class Quad(val tl: Pt, val tr: Pt, val br: Pt, val bl: Pt) {
    val points: List<Pt> get() = listOf(tl, tr, br, bl)

    fun scaled(sx: Double, sy: Double) =
        Quad(Pt(tl.x * sx, tl.y * sy), Pt(tr.x * sx, tr.y * sy), Pt(br.x * sx, br.y * sy), Pt(bl.x * sx, bl.y * sy))

    /** The same corners with corner [i] (0..3, in [points] order) at [p]. */
    fun with(i: Int, p: Pt): Quad = when (i) {
        0 -> copy(tl = p)
        1 -> copy(tr = p)
        2 -> copy(br = p)
        else -> copy(bl = p)
    }

    /** As [x0, y0, x1, y1, ...] (for Matrix.setPolyToPoly). */
    fun toFloats(): FloatArray = points.flatMap { listOf(it.x.toFloat(), it.y.toFloat()) }.toFloatArray()
}

/** A grayscale picture, 0..255 per pixel, row by row. */
class Gray(val w: Int, val h: Int, val px: IntArray)

/** A filter for a scanned page. */
enum class ScanFilter { ORIGINAL, GRAY, BW }

object ScanMath {
    /** The side the picture is scaled down to before looking for edges. */
    const val WORK_SIDE = 256

    /** Luminance of an ARGB pixel, 0..255. */
    fun luma(argb: Int): Int {
        val r = (argb shr 16) and 0xff
        val g = (argb shr 8) and 0xff
        val b = argb and 0xff
        return (r * 299 + g * 587 + b * 114) / 1000
    }

    /** [argb] ([w] x [h]) as grayscale, each output pixel the mean of the
     *  block it covers, the longer side at most [maxSide]. */
    fun downsample(argb: IntArray, w: Int, h: Int, maxSide: Int = WORK_SIDE): Gray {
        require(w > 0 && h > 0 && argb.size >= w * h) { "bad picture" }
        val step = max(1, (max(w, h) + maxSide - 1) / maxSide)
        val ow = max(1, w / step)
        val oh = max(1, h / step)
        val out = IntArray(ow * oh)
        for (oy in 0 until oh) for (ox in 0 until ow) {
            var sum = 0
            var n = 0
            for (dy in 0 until step) {
                val y = oy * step + dy
                if (y >= h) break
                for (dx in 0 until step) {
                    val x = ox * step + dx
                    if (x >= w) break
                    sum += luma(argb[y * w + x])
                    n++
                }
            }
            out[oy * ow + ox] = if (n == 0) 0 else sum / n
        }
        return Gray(ow, oh, out)
    }

    /** Sobel edge strength (|gx| + |gy|) per pixel; 0 on the border. */
    fun sobel(g: Gray): IntArray {
        val (w, h, p) = Triple(g.w, g.h, g.px)
        val out = IntArray(w * h)
        for (y in 1 until h - 1) for (x in 1 until w - 1) {
            val i = y * w + x
            val gx = (p[i - w + 1] + 2 * p[i + 1] + p[i + w + 1]) - (p[i - w - 1] + 2 * p[i - 1] + p[i + w - 1])
            val gy = (p[i + w - 1] + 2 * p[i + w] + p[i + w + 1]) - (p[i - w - 1] + 2 * p[i - w] + p[i - w + 1])
            out[i] = abs(gx) + abs(gy)
        }
        return out
    }

    /** Where an edge counts: well above the picture's usual strength, and
     *  never below a floor (a flat picture has no edges at all). */
    fun edgeThreshold(mag: IntArray): Int {
        if (mag.isEmpty()) return Int.MAX_VALUE
        var sum = 0.0
        var sq = 0.0
        for (m in mag) {
            sum += m
            sq += m.toDouble() * m
        }
        val mean = sum / mag.size
        val sd = sqrt(max(0.0, sq / mag.size - mean * mean))
        return max(MIN_EDGE, (mean + 1.5 * sd).toInt())
    }

    private const val MIN_EDGE = 80

    /** The page in [argb] ([w] x [h]), in its pixels, or null if there's
     *  no clear one. */
    fun detect(argb: IntArray, w: Int, h: Int): Quad? {
        val g = downsample(argb, w, h)
        val q = detectGray(g) ?: return null
        return q.scaled(w.toDouble() / g.w, h.toDouble() / g.h)
    }

    /** [detect], else the whole picture inset a little. */
    fun detectOrFull(argb: IntArray, w: Int, h: Int): Quad = detect(argb, w, h) ?: full(w, h)

    /** The whole [w] x [h] picture, [inset] (a fraction) in from each side. */
    fun full(w: Int, h: Int, inset: Double = 0.06): Quad {
        val dx = w * inset
        val dy = h * inset
        return Quad(Pt(dx, dy), Pt(w - dx, dy), Pt(w - dx, h - dy), Pt(dx, h - dy))
    }

    /** The page in a grayscale picture, in its pixels. */
    fun detectGray(g: Gray): Quad? {
        val mag = sobel(g)
        val t = edgeThreshold(mag)
        val edge = BooleanArray(mag.size) { mag[it] >= t }
        val seen = BooleanArray(mag.size)
        val total = g.w.toDouble() * g.h
        var best: List<Pt>? = null
        var bestArea = 0.0
        val stack = IntArray(mag.size)
        for (start in mag.indices) {
            if (!edge[start] || seen[start]) continue
            // One outline: the edge pixels connected to this one.
            val pts = ArrayList<Pt>()
            var top = 0
            stack[top++] = start
            seen[start] = true
            while (top > 0) {
                val i = stack[--top]
                val x = i % g.w
                val y = i / g.w
                pts.add(Pt(x.toDouble(), y.toDouble()))
                for (dy in -1..1) for (dx in -1..1) {
                    val nx = x + dx
                    val ny = y + dy
                    if (nx < 0 || ny < 0 || nx >= g.w || ny >= g.h) continue
                    val j = ny * g.w + nx
                    if (edge[j] && !seen[j]) {
                        seen[j] = true
                        stack[top++] = j
                    }
                }
            }
            if (pts.size < 16) continue
            val hull = convexHull(pts)
            if (hull.size < 4) continue
            val quad = largestQuad(hull)
            val area = polygonArea(quad)
            // A page: a good part of the picture, and its outline is a
            // quadrilateral (the four corners cover most of the hull).
            if (area < total * MIN_PAGE || area > total * MAX_PAGE) continue
            if (area < polygonArea(hull) * MIN_FILL) continue
            if (area > bestArea) {
                bestArea = area
                best = quad
            }
        }
        return best?.let { order(it) }
    }

    private const val MIN_PAGE = 0.10
    private const val MAX_PAGE = 0.995
    private const val MIN_FILL = 0.85

    /** The convex hull, counter-clockwise in screen terms (monotone chain). */
    fun convexHull(points: List<Pt>): List<Pt> {
        val p = points.distinct().sortedWith(compareBy({ it.x }, { it.y }))
        if (p.size < 3) return p
        fun cross(o: Pt, a: Pt, b: Pt) = (a.x - o.x) * (b.y - o.y) - (a.y - o.y) * (b.x - o.x)
        val lower = ArrayList<Pt>()
        for (pt in p) {
            while (lower.size >= 2 && cross(lower[lower.size - 2], lower[lower.size - 1], pt) <= 0) lower.removeAt(lower.size - 1)
            lower.add(pt)
        }
        val upper = ArrayList<Pt>()
        for (pt in p.asReversed()) {
            while (upper.size >= 2 && cross(upper[upper.size - 2], upper[upper.size - 1], pt) <= 0) upper.removeAt(upper.size - 1)
            upper.add(pt)
        }
        lower.removeAt(lower.size - 1)
        upper.removeAt(upper.size - 1)
        return lower + upper
    }

    /** The four points of convex polygon [hull] enclosing the most area
     *  (a diagonal and the farthest point on each side of it). A long hull
     *  is thinned first, so this stays quick. */
    fun largestQuad(hull: List<Pt>): List<Pt> {
        val h = if (hull.size > MAX_HULL) List(MAX_HULL) { hull[it * hull.size / MAX_HULL] } else hull
        val n = h.size
        if (n <= 4) return h
        var best = listOf(h[0], h[1], h[2], h[3])
        var bestArea = -1.0
        for (i in 0 until n) for (k in i + 2 until n) {
            var a1 = -1.0
            var j1 = -1
            for (j in i + 1 until k) {
                val a = abs(tri(h[i], h[j], h[k]))
                if (a > a1) { a1 = a; j1 = j }
            }
            var a2 = -1.0
            var l2 = -1
            for (l in k + 1 until n + i) {
                val a = abs(tri(h[i], h[l % n], h[k]))
                if (a > a2) { a2 = a; l2 = l % n }
            }
            if (j1 < 0 || l2 < 0) continue
            if (a1 + a2 > bestArea) {
                bestArea = a1 + a2
                best = listOf(h[i], h[j1], h[k], h[l2])
            }
        }
        return best
    }

    private const val MAX_HULL = 120

    private fun tri(a: Pt, b: Pt, c: Pt) = ((b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x)) / 2

    /** The area of a simple polygon (shoelace). */
    fun polygonArea(p: List<Pt>): Double {
        var s = 0.0
        for (i in p.indices) {
            val a = p[i]
            val b = p[(i + 1) % p.size]
            s += a.x * b.y - b.x * a.y
        }
        return abs(s) / 2
    }

    /** Four corners in page order: top left (smallest x+y), bottom right
     *  (largest x+y), top right (largest x-y), bottom left (smallest x-y). */
    fun order(p: List<Pt>): Quad {
        require(p.size == 4) { "four corners" }
        val idx = p.indices.toMutableList()
        val tl = idx.minByOrNull { p[it].x + p[it].y }!!
        idx.remove(tl)
        val br = idx.maxByOrNull { p[it].x + p[it].y }!!
        idx.remove(br)
        val tr = idx.maxByOrNull { p[it].x - p[it].y }!!
        idx.remove(tr)
        return Quad(p[tl], p[tr], p[br], p[idx[0]])
    }

    /** Whether [q]'s corners make a convex shape, turning one way. */
    fun isConvex(q: Quad): Boolean {
        val p = q.points
        var sign = 0
        for (i in 0 until 4) {
            val c = tri(p[i], p[(i + 1) % 4], p[(i + 2) % 4])
            if (c == 0.0) return false
            val s = if (c > 0) 1 else -1
            if (sign == 0) sign = s else if (s != sign) return false
        }
        return true
    }

    // --- perspective ---------------------------------------------------------

    /** The 3x3 homography (row by row, h[8] = 1) taking each of [src] to
     *  the matching [dst] (four points each). */
    fun homography(src: List<Pt>, dst: List<Pt>): DoubleArray {
        require(src.size == 4 && dst.size == 4) { "four points each" }
        val a = Array(8) { DoubleArray(9) }
        for (i in 0 until 4) {
            val (x, y) = src[i]
            val (u, v) = dst[i]
            a[2 * i] = doubleArrayOf(x, y, 1.0, 0.0, 0.0, 0.0, -u * x, -u * y, u)
            a[2 * i + 1] = doubleArrayOf(0.0, 0.0, 0.0, x, y, 1.0, -v * x, -v * y, v)
        }
        // Gaussian elimination with partial pivoting.
        for (c in 0 until 8) {
            val piv = (c until 8).maxByOrNull { abs(a[it][c]) }!!
            require(abs(a[piv][c]) > 1e-12) { "three corners in a line" }
            val t = a[c]; a[c] = a[piv]; a[piv] = t
            for (r in 0 until 8) {
                if (r == c) continue
                val f = a[r][c] / a[c][c]
                if (f == 0.0) continue
                for (k in c..8) a[r][k] -= f * a[c][k]
            }
        }
        val h = DoubleArray(9)
        for (i in 0 until 8) h[i] = a[i][8] / a[i][i]
        h[8] = 1.0
        return h
    }

    /** [p] through homography [h]. */
    fun map(h: DoubleArray, p: Pt): Pt {
        val w = h[6] * p.x + h[7] * p.y + h[8]
        return Pt((h[0] * p.x + h[1] * p.y + h[2]) / w, (h[3] * p.x + h[4] * p.y + h[5]) / w)
    }

    private fun dist(a: Pt, b: Pt) = hypot(a.x - b.x, a.y - b.y)

    /** How big the straightened page comes out: the longer of its top and
     *  bottom edges wide, the longer of its sides high (at most [maxSide]
     *  on its longer side, keeping the shape). */
    fun outputSize(q: Quad, maxSide: Int = 3000): Pair<Int, Int> {
        var w = max(dist(q.tl, q.tr), dist(q.bl, q.br))
        var h = max(dist(q.tl, q.bl), dist(q.tr, q.br))
        val s = max(w, h)
        if (s > maxSide) {
            w = w * maxSide / s
            h = h * maxSide / s
        }
        return max(1, w.toInt()) to max(1, h.toInt())
    }

    // --- black and white -------------------------------------------------------

    /** Black (0) where a pixel is darker than the mean of the
     *  (2 [radius] + 1)² square around it by more than [c], else white
     *  (255). Uneven light and shadows don't matter: each pixel is judged
     *  against its own neighbourhood. */
    fun adaptiveThreshold(gray: IntArray, w: Int, h: Int, radius: Int = 15, c: Int = 10): IntArray {
        require(gray.size >= w * h) { "bad picture" }
        // Integral image, one larger each way.
        val iw = w + 1
        val sum = LongArray(iw * (h + 1))
        for (y in 0 until h) {
            var row = 0L
            for (x in 0 until w) {
                row += gray[y * w + x]
                sum[(y + 1) * iw + x + 1] = sum[y * iw + x + 1] + row
            }
        }
        val out = IntArray(w * h)
        for (y in 0 until h) {
            val y0 = max(0, y - radius)
            val y1 = min(h - 1, y + radius)
            for (x in 0 until w) {
                val x0 = max(0, x - radius)
                val x1 = min(w - 1, x + radius)
                val n = (x1 - x0 + 1).toLong() * (y1 - y0 + 1)
                val s = sum[(y1 + 1) * iw + x1 + 1] - sum[y0 * iw + x1 + 1] - sum[(y1 + 1) * iw + x0] + sum[y0 * iw + x0]
                out[y * w + x] = if (gray[y * w + x] * n < s - c.toLong() * n) 0 else 255
            }
        }
        return out
    }

    /** ARGB pixels with [filter] applied (a new array; ORIGINAL copies). */
    fun applyFilter(argb: IntArray, w: Int, h: Int, filter: ScanFilter): IntArray = when (filter) {
        ScanFilter.ORIGINAL -> argb.copyOf()
        ScanFilter.GRAY -> IntArray(w * h) { val l = luma(argb[it]); grayArgb(l) }
        ScanFilter.BW -> {
            val g = IntArray(w * h) { luma(argb[it]) }
            // About a fortieth of the page: big enough to span a letter.
            val r = max(4, min(w, h) / 40)
            adaptiveThreshold(g, w, h, r, 10).let { t -> IntArray(w * h) { grayArgb(t[it]) } }
        }
    }

    private fun grayArgb(l: Int) = (0xff shl 24) or (l shl 16) or (l shl 8) or l
}

/** The scanned pages, in order. Every change returns a new list. */
object ScanPages {
    /** [list] with the item at [from] moved to [to] (clamped). */
    fun <T> move(list: List<T>, from: Int, to: Int): List<T> {
        if (from !in list.indices) return list
        val t = to.coerceIn(0, list.size - 1)
        if (t == from) return list
        val m = list.toMutableList()
        val item = m.removeAt(from)
        m.add(t, item)
        return m
    }

    fun <T> up(list: List<T>, i: Int) = move(list, i, i - 1)

    fun <T> down(list: List<T>, i: Int) = move(list, i, i + 1)

    fun <T> delete(list: List<T>, i: Int): List<T> =
        if (i in list.indices) list.filterIndexed { j, _ -> j != i } else list
}

/** A PDF page size, in points (1/72 inch). */
data class Paper(val w: Double, val h: Double) {
    val landscape get() = Paper(max(w, h), min(w, h))

    companion object {
        val A4 = Paper(595.0, 842.0)
        val LETTER = Paper(612.0, 792.0)

        /** Letter in the US and Canada (and a few others), A4 elsewhere. */
        fun forCountry(country: String): Paper =
            if (country.uppercase() in setOf("US", "CA", "MX", "PH", "CL", "CO", "VE", "GT", "PR")) LETTER else A4
    }
}

/** Where a picture goes on a page: its top left and size, in points. */
data class Placed(val x: Double, val y: Double, val w: Double, val h: Double)

object PaperFit {
    /** The page for a [w] x [h] picture: [paper] upright, or turned for a
     *  wide picture. */
    fun pageFor(w: Int, h: Int, paper: Paper): Paper = if (w > h) paper.landscape else paper

    /** The picture as large as fits inside [margin] on [page], its shape
     *  kept, centred. */
    fun fit(w: Int, h: Int, page: Paper, margin: Double = 28.0): Placed {
        val aw = page.w - 2 * margin
        val ah = page.h - 2 * margin
        val s = min(aw / w, ah / h)
        val pw = w * s
        val ph = h * s
        return Placed((page.w - pw) / 2, (page.h - ph) / 2, pw, ph)
    }
}
