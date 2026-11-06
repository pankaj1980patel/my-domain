package com.mydomain.android.core

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import kotlin.math.abs
import kotlin.math.cos
import kotlin.math.hypot
import kotlin.math.sin

class ScanMathTest {
    private val dark = 0xff202020.toInt()
    private val light = 0xfff0f0f0.toInt()

    /** A [w] x [h] picture, [bg] everywhere and [fg] inside polygon [poly]. */
    private fun draw(w: Int, h: Int, poly: List<Pt>, bg: Int = dark, fg: Int = light): IntArray {
        val px = IntArray(w * h) { bg }
        for (y in 0 until h) for (x in 0 until w) {
            if (inside(poly, x + 0.5, y + 0.5)) px[y * w + x] = fg
        }
        return px
    }

    private fun inside(poly: List<Pt>, x: Double, y: Double): Boolean {
        var c = false
        var j = poly.size - 1
        for (i in poly.indices) {
            val a = poly[i]
            val b = poly[j]
            if ((a.y > y) != (b.y > y) && x < (b.x - a.x) * (y - a.y) / (b.y - a.y) + a.x) c = !c
            j = i
        }
        return c
    }

    private fun near(want: Pt, got: Pt, tol: Double) {
        assertTrue("wanted $want, got $got", hypot(want.x - got.x, want.y - got.y) <= tol)
    }

    /** A page-like quadrilateral turned by [deg] about the picture's centre. */
    private fun turned(w: Int, h: Int, deg: Double): List<Pt> {
        val cx = w / 2.0
        val cy = h / 2.0
        val r = Math.toRadians(deg)
        return listOf(Pt(-180.0, -240.0), Pt(180.0, -240.0), Pt(180.0, 240.0), Pt(-180.0, 240.0)).map {
            Pt(cx + it.x * cos(r) - it.y * sin(r), cy + it.x * sin(r) + it.y * cos(r))
        }
    }

    @Test
    fun aPageTurnedOnADarkTableIsFoundCornersInOrder() {
        val (w, h) = 640 to 800
        val page = turned(w, h, 12.0)
        val q = ScanMath.detect(draw(w, h, page), w, h)
        assertNotNull(q)
        q!!
        // Turned clockwise by 12°: the corners stay top left, top right...
        // within a few pixels of the drawn ones (the work picture is 256 px).
        val tol = 12.0
        near(page[0], q.tl, tol)
        near(page[1], q.tr, tol)
        near(page[2], q.br, tol)
        near(page[3], q.bl, tol)
        assertTrue(ScanMath.isConvex(q))
    }

    @Test
    fun aSkewedPageInPerspectiveIsFound() {
        val (w, h) = 600 to 600
        val page = listOf(Pt(150.0, 90.0), Pt(470.0, 120.0), Pt(540.0, 520.0), Pt(70.0, 480.0))
        val q = ScanMath.detect(draw(w, h, page), w, h)!!
        q.points.zip(page).forEach { (got, want) -> near(want, got, 10.0) }
    }

    @Test
    fun aDarkPageOnALightDeskIsFoundToo() {
        val (w, h) = 500 to 500
        val page = listOf(Pt(100.0, 80.0), Pt(400.0, 100.0), Pt(420.0, 420.0), Pt(90.0, 400.0))
        val q = ScanMath.detect(draw(w, h, page, bg = light, fg = dark), w, h)!!
        q.points.zip(page).forEach { (got, want) -> near(want, got, 10.0) }
    }

    @Test
    fun noPageMeansTheWholePictureInsetForYouToDrag() {
        val (w, h) = 400 to 300
        val flat = IntArray(w * h) { dark }
        assertNull(ScanMath.detect(flat, w, h))
        val q = ScanMath.detectOrFull(flat, w, h)
        assertEquals(ScanMath.full(w, h), q)
        assertTrue(q.tl.x > 0 && q.tl.y > 0 && q.br.x < w && q.br.y < h)
        // A speck is not a page.
        val speck = draw(w, h, listOf(Pt(10.0, 10.0), Pt(30.0, 10.0), Pt(30.0, 30.0), Pt(10.0, 30.0)))
        assertNull(ScanMath.detect(speck, w, h))
    }

    @Test
    fun cornersAreOrderedWhateverOrderTheyComeIn() {
        val pts = listOf(Pt(90.0, 95.0), Pt(10.0, 12.0), Pt(8.0, 88.0), Pt(95.0, 9.0))
        val q = ScanMath.order(pts)
        assertEquals(Pt(10.0, 12.0), q.tl)
        assertEquals(Pt(95.0, 9.0), q.tr)
        assertEquals(Pt(90.0, 95.0), q.br)
        assertEquals(Pt(8.0, 88.0), q.bl)
        assertTrue(ScanMath.isConvex(q))
        // Two corners swapped by a drag: a bow tie, not convex.
        assertFalse(ScanMath.isConvex(q.copy(tr = q.br, br = q.tr)))
    }

    @Test
    fun theLargestQuadOfAHullIsItsCorners() {
        // A square with points along its edges: the corners win.
        val pts = mutableListOf<Pt>()
        for (i in 0..10) {
            pts += Pt(i * 10.0, 0.0); pts += Pt(i * 10.0, 100.0)
            pts += Pt(0.0, i * 10.0); pts += Pt(100.0, i * 10.0)
        }
        val hull = ScanMath.convexHull(pts)
        assertEquals(4, hull.size)
        val octagon = (0 until 8).map { Pt(50 + 50 * cos(it * Math.PI / 4), 50 + 50 * sin(it * Math.PI / 4)) }
        val quad = ScanMath.largestQuad(ScanMath.convexHull(octagon))
        assertEquals(5000.0, ScanMath.polygonArea(quad), 1e-6)
    }

    @Test
    fun theHomographyTakesEachCornerHomeAndKeepsLinesStraight() {
        val src = listOf(Pt(120.0, 80.0), Pt(500.0, 130.0), Pt(560.0, 620.0), Pt(60.0, 560.0))
        val dst = listOf(Pt(0.0, 0.0), Pt(400.0, 0.0), Pt(400.0, 600.0), Pt(0.0, 600.0))
        val h = ScanMath.homography(src, dst)
        src.zip(dst).forEach { (s, d) ->
            val m = ScanMath.map(h, s)
            assertEquals(d.x, m.x, 1e-6)
            assertEquals(d.y, m.y, 1e-6)
        }
        // The crossing of the diagonals goes to the page's centre.
        val c = cross(src[0], src[2], src[1], src[3])
        val m = ScanMath.map(h, c)
        assertEquals(200.0, m.x, 1e-6)
        assertEquals(300.0, m.y, 1e-6)
        // A point on the top edge stays on the top edge.
        val top = Pt((src[0].x + src[1].x) / 2, (src[0].y + src[1].y) / 2)
        assertEquals(0.0, ScanMath.map(h, top).y, 1e-6)
        // The square to itself: nothing moves.
        val sq = listOf(Pt(0.0, 0.0), Pt(1.0, 0.0), Pt(1.0, 1.0), Pt(0.0, 1.0))
        val id = ScanMath.map(ScanMath.homography(sq, sq), Pt(0.3, 0.7))
        assertEquals(0.3, id.x, 1e-9)
        assertEquals(0.7, id.y, 1e-9)
    }

    private fun cross(a: Pt, b: Pt, c: Pt, d: Pt): Pt {
        val den = (a.x - b.x) * (c.y - d.y) - (a.y - b.y) * (c.x - d.x)
        val t = ((a.x - c.x) * (c.y - d.y) - (a.y - c.y) * (c.x - d.x)) / den
        return Pt(a.x + t * (b.x - a.x), a.y + t * (b.y - a.y))
    }

    @Test(expected = IllegalArgumentException::class)
    fun threeCornersInALineAreRefused() {
        val line = listOf(Pt(0.0, 0.0), Pt(1.0, 0.0), Pt(2.0, 0.0), Pt(0.0, 1.0))
        ScanMath.homography(line, line.map { Pt(it.x * 2, it.y * 2) })
    }

    @Test
    fun theStraightenedPageTakesItsLongestEdges() {
        val q = Quad(Pt(0.0, 0.0), Pt(300.0, 10.0), Pt(320.0, 410.0), Pt(10.0, 400.0))
        val (w, h) = ScanMath.outputSize(q)
        assertEquals(310, w) // bottom edge 310.0 vs top 300.2
        assertEquals(400, h) // right side 400.5 vs left 400.1
        // Capped on its longer side, its shape kept.
        val (cw, ch) = ScanMath.outputSize(Quad(Pt(0.0, 0.0), Pt(6000.0, 0.0), Pt(6000.0, 3000.0), Pt(0.0, 3000.0)))
        assertEquals(3000, cw)
        assertEquals(1500, ch)
    }

    @Test
    fun blackAndWhiteKeepsTextThroughUnevenLight() {
        val (w, h) = 120 to 60
        // Light falling from 240 on the left to 110 on the right, with two
        // dark strokes 60 below the light where they are.
        val g = IntArray(w * h) { i ->
            val x = i % w
            val y = i / w
            val bg = 240 - x * 130 / w
            val stroke = (x in 20..22 || x in 90..92) && y in 10..50
            if (stroke) bg - 60 else bg
        }
        val bw = ScanMath.adaptiveThreshold(g, w, h, radius = 8, c = 10)
        assertTrue(bw.all { it == 0 || it == 255 })
        // The strokes are black, both in the bright and the dim part...
        assertEquals(0, bw[30 * w + 21])
        assertEquals(0, bw[30 * w + 91])
        // ...and the page around them white, even where it's darker than
        // the bright stroke (a global threshold would get one of them wrong).
        assertEquals(255, bw[30 * w + 60])
        assertEquals(255, bw[30 * w + 110])
        assertTrue(g[30 * w + 110] < g[30 * w + 21])
        val black = bw.count { it == 0 }
        assertTrue("strokes only, got $black black", black in 2 * 3 * 41..2 * 3 * 41 + 20)
    }

    @Test
    fun filtersMakeGrayAndBlackAndWhitePixels() {
        val px = intArrayOf(0xffff0000.toInt(), 0xff00ff00.toInt(), 0xff0000ff.toInt(), 0xffffffff.toInt())
        val gray = ScanMath.applyFilter(px, 2, 2, ScanFilter.GRAY)
        gray.forEach { p ->
            assertEquals((p shr 16) and 0xff, p and 0xff)
            assertEquals((p shr 8) and 0xff, p and 0xff)
        }
        assertEquals(76, gray[0] and 0xff)
        assertTrue(ScanMath.applyFilter(px, 2, 2, ScanFilter.ORIGINAL).contentEquals(px))
        ScanMath.applyFilter(px, 2, 2, ScanFilter.BW).forEach { assertTrue(it == 0xff000000.toInt() || it == -1) }
    }

    @Test
    fun pagesMoveAndGoAwayInOrder() {
        val pages = listOf("a", "b", "c", "d")
        assertEquals(listOf("b", "a", "c", "d"), ScanPages.up(pages, 1))
        assertEquals(pages, ScanPages.up(pages, 0)) // the first can't go higher
        assertEquals(listOf("a", "b", "d", "c"), ScanPages.down(pages, 2))
        assertEquals(pages, ScanPages.down(pages, 3))
        assertEquals(listOf("b", "c", "a", "d"), ScanPages.move(pages, 0, 2))
        assertEquals(listOf("a", "c", "d"), ScanPages.delete(pages, 1))
        assertEquals(pages, ScanPages.delete(pages, 9))
        assertEquals(emptyList<String>(), ScanPages.delete(listOf("a"), 0))
        assertEquals(pages, ScanPages.move(pages, -1, 2))
    }

    @Test
    fun aPageFitsItsPaperInsideTheMarginsKeepingItsShape() {
        assertEquals(Paper.LETTER, Paper.forCountry("us"))
        assertEquals(Paper.LETTER, Paper.forCountry("CA"))
        assertEquals(Paper.A4, Paper.forCountry("IN"))
        assertEquals(Paper.A4, Paper.forCountry(""))
        // A tall page on A4.
        val page = PaperFit.pageFor(1000, 1414, Paper.A4)
        assertEquals(Paper.A4, page)
        val p = PaperFit.fit(1000, 1414, page, margin = 28.0)
        assertTrue(p.x >= 28 - 1e-9 && p.y >= 28 - 1e-9)
        assertTrue(p.x + p.w <= 595 - 28 + 1e-9 && p.y + p.h <= 842 - 28 + 1e-9)
        assertEquals(1000.0 / 1414, p.w / p.h, 1e-9)
        assertTrue(abs(p.x * 2 + p.w - 595) < 1e-9) // centred
        // A wide picture gets the paper turned.
        val wide = PaperFit.pageFor(1600, 900, Paper.LETTER)
        assertEquals(Paper(792.0, 612.0), wide)
        val q = PaperFit.fit(1600, 900, wide)
        assertEquals(1600.0 / 900, q.w / q.h, 1e-9)
        assertEquals(792.0 - 56, q.w, 1e-9)
    }
}
