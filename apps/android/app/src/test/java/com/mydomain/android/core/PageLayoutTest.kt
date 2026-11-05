package com.mydomain.android.core

import org.junit.Assert.assertEquals
import org.junit.Assert.assertSame
import org.junit.Assert.assertTrue
import org.junit.Test

class PageLayoutTest {

    private val p = PageArrangement

    @Test
    fun nothingSavedIsTheDefaultOrder() {
        val tiles = listOf("chat", "files", "info")
        assertEquals(PageLayout(tiles, emptyList()), p.merge(PageLayout(), tiles))
    }

    @Test
    fun savedOrderWins() {
        val saved = PageLayout(listOf("info", "chat", "files"), listOf("files"))
        assertEquals(saved, p.merge(saved, listOf("chat", "files", "info")))
    }

    @Test
    fun aNewTileLandsAfterItsDefaultNeighbour() {
        val tiles = listOf("chat", "files", "info", "connection", "apps")
        val saved = PageLayout(listOf("info", "chat", "files", "apps"))
        assertEquals(listOf("info", "connection", "chat", "files", "apps"), p.merge(saved, tiles).order)
    }

    @Test
    fun aNewFirstTileGoesFirst() {
        val saved = PageLayout(listOf("files", "chat"))
        assertEquals(listOf("new", "files", "chat"), p.merge(saved, listOf("new", "chat", "files")).order)
    }

    @Test
    fun severalNewTilesKeepTheirDefaultOrder() {
        val saved = PageLayout(listOf("e", "a"))
        assertEquals(listOf("e", "a", "b", "c", "d"), p.merge(saved, listOf("a", "b", "c", "d", "e")).order)
    }

    @Test
    fun goneAndRepeatedIdsAreSkipped() {
        val saved = PageLayout(listOf("old", "files", "files", "chat"), listOf("old", "chat", "chat"))
        assertEquals(PageLayout(listOf("files", "chat"), listOf("chat")), p.merge(saved, listOf("chat", "files")))
    }

    @Test
    fun everyTileComesBackOnce() {
        val tiles = listOf("a", "b", "c", "d")
        for (saved in listOf(
            PageLayout(),
            PageLayout(listOf("d")),
            PageLayout(listOf("c", "x", "a"), listOf("x")),
            PageLayout(listOf("d", "c", "b", "a")),
        )) {
            assertEquals(tiles, p.merge(saved, tiles).order.sorted())
        }
    }

    @Test
    fun cleanDropsBadIds() {
        val long = "x".repeat(PageArrangement.MAX_ID + 1)
        val dirty = PageLayout(listOf("a", " ", long, "a", "b"), listOf(long, "b", "b"))
        assertEquals(PageLayout(listOf("a", "b"), listOf("b")), p.clean(dirty))
        val many = PageLayout((0 until PageArrangement.MAX_TILES + 10).map { it.toString() })
        assertEquals(PageArrangement.MAX_TILES, p.clean(many).order.size)
    }

    @Test
    fun movingOntoAnother() {
        val order = listOf("a", "b", "c", "d")
        assertEquals(listOf("b", "c", "a", "d"), p.moveOnto(order, "a", "c"))
        assertEquals(listOf("d", "a", "b", "c"), p.moveOnto(order, "d", "a"))
        assertSame(order, p.moveOnto(order, "a", "a"))
        assertSame(order, p.moveOnto(order, "a", "zz"))
    }

    @Test
    fun steppingSkipsHiddenTiles() {
        val order = listOf("a", "b", "c", "d")
        val hidden = listOf("b")
        assertEquals(listOf("b", "c", "a", "d"), p.moveStep(order, hidden, "a", 1))
        assertEquals(listOf("c", "a", "b", "d"), p.moveStep(order, hidden, "c", -1))
        // At either end: unchanged.
        assertSame(order, p.moveStep(order, hidden, "a", -1))
        assertSame(order, p.moveStep(order, hidden, "d", 1))
    }

    @Test
    fun deviceLayoutWinsOverKind() {
        val tiles = listOf("a", "b", "c")
        var s = SavedLayouts()
        assertEquals(LayoutScope.DEFAULT, p.view(s, "phone", "n1", tiles).scope)

        s = p.apply(s, "phone", "n1", false, PageLayout(listOf("c", "b", "a")))
        p.view(s, "phone", "n2", tiles).let {
            assertEquals(LayoutScope.KIND, it.scope)
            assertEquals(listOf("c", "b", "a"), it.order)
        }
        // A computer isn't affected.
        assertEquals(LayoutScope.DEFAULT, p.view(s, "computer", "n3", tiles).scope)

        s = p.apply(s, "phone", "n1", true, PageLayout(listOf("b", "a", "c"), listOf("c")))
        p.view(s, "phone", "n1", tiles).let {
            assertEquals(LayoutScope.DEVICE, it.scope)
            assertEquals(listOf("b", "a", "c"), it.order)
            assertEquals(listOf("b", "a"), it.shown)
        }
        // Other phones keep the kind's.
        assertEquals(listOf("c", "b", "a"), p.view(s, "phone", "n2", tiles).order)
    }

    @Test
    fun resetting() {
        val tiles = listOf("a", "b")
        var s = SavedLayouts()
        s = p.apply(s, "phone", "n1", false, PageLayout(listOf("b", "a")))
        s = p.apply(s, "phone", "n1", true, PageLayout(listOf("a", "b"), listOf("a")))
        // Reset "only for this device": back to the phone layout.
        s = p.apply(s, "phone", "n1", true, null)
        p.view(s, "phone", "n1", tiles).let {
            assertEquals(LayoutScope.KIND, it.scope)
            assertEquals(listOf("b", "a"), it.order)
        }
        // Its own again, then reset for every phone: all gone.
        s = p.apply(s, "phone", "n1", true, PageLayout(listOf("a", "b")))
        s = p.apply(s, "phone", "n1", false, null)
        assertEquals(SavedLayouts(), s)
    }

    @Test
    fun savingForTheKindDropsTheDeviceLayout() {
        var s = p.apply(SavedLayouts(), "computer", "n1", true, PageLayout(listOf("b", "a")))
        s = p.apply(s, "computer", "n1", false, PageLayout(listOf("a", "b"), listOf("b")))
        val v = p.view(s, "computer", "n1", listOf("a", "b"))
        assertEquals(LayoutScope.KIND, v.scope)
        assertEquals(listOf("b"), v.hidden)
        assertTrue(s.devices.isEmpty())
    }

    @Test
    fun unknownKindIsAComputer() {
        val s = p.apply(SavedLayouts(), "toaster", "n1", false, PageLayout(listOf("b", "a")))
        assertEquals(listOf("b", "a"), p.view(s, "computer", "n2", listOf("a", "b")).order)
    }

    @Test
    fun deviceLayoutsAreCapped() {
        var s = SavedLayouts()
        for (i in 0 until PageArrangement.MAX_DEVICES + 5) s = p.apply(s, "phone", "n$i", true, PageLayout(listOf("a")))
        assertEquals(PageArrangement.MAX_DEVICES, s.devices.size)
        // One already kept can still change.
        s = p.apply(s, "phone", "n0", true, PageLayout(listOf("b")))
        assertEquals(listOf("b"), s.devices["n0"]?.order)
    }
}
