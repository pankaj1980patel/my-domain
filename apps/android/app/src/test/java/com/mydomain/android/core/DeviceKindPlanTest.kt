package com.mydomain.android.core

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class DeviceKindPlanTest {

    private val p = DeviceKindPlan

    @Test
    fun formFromScreenAndFeatures() {
        assertEquals("phone", p.form(411, hinge = false, leanback = false))
        assertEquals("phone", p.form(599, hinge = false, leanback = false))
        assertEquals("tablet", p.form(600, hinge = false, leanback = false))
        assertEquals("tablet", p.form(800, hinge = false, leanback = false))
        // A foldable is a foldable open or closed (its width changes).
        assertEquals("foldable", p.form(673, hinge = true, leanback = false))
        assertEquals("foldable", p.form(360, hinge = true, leanback = false))
        assertEquals("tv", p.form(540, hinge = false, leanback = true))
    }

    @Test
    fun makerCodesAreSpotted() {
        for (code in listOf("I2405", "SM-S918B", "CPH2451", "23049PCD8G", "V2318", "RMX3771", "M2101K6G", "GT-I9505")) {
            assertTrue(code, p.looksLikeCode(code))
        }
        for (name in listOf("Pixel 8", "moto g54 5G", "Galaxy S23", "Nokia G21", "")) {
            assertFalse(name, p.looksLikeCode(name))
        }
    }

    @Test
    fun theMarketingNameReplacesACode() {
        val vivo = p.tidyMaker("vivo")
        assertEquals("T3 Ultra", p.displayModel(vivo, "I2405", "vivo T3 Ultra"))
        assertEquals("Redmi Note 12", p.displayModel("Xiaomi", "23049PCD8G", "Redmi Note 12"))
        // A name people know stays.
        assertEquals("Pixel 8", p.displayModel("Google", "Pixel 8", "Pankaj's Pixel"))
        // A name the user typed isn't a model.
        assertEquals("SM-S918B", p.displayModel("Samsung", "SM-S918B", "Pankaj’s S23"))
        assertEquals("SM-S918B", p.displayModel("Samsung", "SM-S918B", "Pankaj's phone"))
        assertEquals("SM-S918B", p.displayModel("Samsung", "SM-S918B", null))
        assertEquals("SM-S918B", p.displayModel("Samsung", "SM-S918B", "  "))
        assertEquals("I2405", p.displayModel("Vivo", "I2405", "I2405"))
    }

    @Test
    fun makersAreTidied() {
        assertEquals("Samsung", p.tidyMaker("samsung"))
        assertEquals("Huawei", p.tidyMaker("HUAWEI"))
        assertEquals("Google", p.tidyMaker("Google"))
        assertEquals("iQOO", p.tidyMaker("iqoo"))
        assertEquals("OnePlus", p.tidyMaker("OnePlus"))
        assertEquals("OnePlus", p.tidyMaker("ONEPLUS"))
        assertEquals("Vivo", p.tidyMaker("vivo"))
        assertEquals("", p.tidyMaker(""))
    }

    @Test
    fun descriptionsDontRepeatTheMaker() {
        assertEquals("Samsung Galaxy S23", p.describe("Samsung", "Galaxy S23"))
        assertEquals("Apple MacBook Pro", p.describe("Apple", "MacBook Pro"))
        assertEquals("Lenovo ThinkPad X1", p.describe("Lenovo", "Lenovo ThinkPad X1"))
        assertEquals("Pixel 8", p.describe("", "Pixel 8"))
        assertEquals("Dell", p.describe("Dell", ""))
        assertEquals("", p.describe("", ""))
        assertEquals("Apple MacBook Pro", DeviceKind(maker = "Apple", model = "MacBook Pro").description)
    }

    @Test
    fun thisPhonesIcon() {
        assertEquals("android_phone", p.androidIcon("phone"))
        assertEquals("android_tablet", p.androidIcon("tablet"))
        assertEquals("android_foldable", p.androidIcon("foldable"))
        assertEquals("tv", p.androidIcon("tv"))
        assertEquals("android_phone", p.androidIcon(""))
    }

    @Test
    fun iconChoice() {
        val mac = DeviceKind(os = "macos", form = "laptop", icon = "mac_laptop")
        // What it said.
        assertEquals("mac_laptop", p.iconOf(null, mac))
        // The icon you chose wins.
        assertEquals("tv", p.iconOf("tv", mac))
        assertEquals("win_desktop", p.iconOf("win_desktop", mac))
        // A chosen icon this app can't draw is ignored.
        assertEquals("mac_laptop", p.iconOf("toaster", mac))
        // An older app that never said: unknown (or the label).
        assertEquals("unknown", p.iconOf(null, null))
        assertEquals("unknown", p.iconOf(null, DeviceKind()))
        assertEquals("phone", p.iconOf("phone", null))
        assertEquals("unknown", p.iconOf(null, DeviceKind(icon = "rocket")))
    }

    @Test
    fun textIsTidied() {
        assertEquals("Galaxy S23", p.tidy("  Galaxy\n S23 "))
        assertEquals(60, p.tidy("x".repeat(100)).length)
    }

    @Test
    fun everyIconHasADrawing() {
        val s = DeviceIconShapes
        assertEquals(17, s.ICONS.size)
        assertEquals(s.ICONS.size, s.ICONS.toSet().size)
        for (icon in s.ICONS) {
            val shape = s.shape(icon)
            assertTrue(icon, shape.parts.isNotEmpty())
            shape.mark?.let { assertTrue(icon, s.markParts(it.kind).isNotEmpty()) }
        }
        // OS marks: an apple on Macs, panes on Windows, a penguin on Linux,
        // a robot head on Android.
        assertEquals("apple", s.shape("mac_laptop").mark?.kind)
        assertEquals("apple", s.shape("imac").mark?.kind)
        assertEquals("apple", s.shape("mac_desktop").mark?.kind)
        assertEquals("windows", s.shape("win_laptop").mark?.kind)
        assertEquals("windows", s.shape("win_desktop").mark?.kind)
        assertEquals("tux", s.shape("linux_laptop").mark?.kind)
        assertEquals("tux", s.shape("linux_desktop").mark?.kind)
        assertEquals("android", s.shape("android_phone").mark?.kind)
        assertEquals("android", s.shape("android_tablet").mark?.kind)
        assertEquals(null, s.shape("laptop").mark)
        // Anything else draws "unknown".
        assertEquals(s.shape("unknown"), s.shape("toaster"))
    }

    @Test
    fun pathDataIsWellFormed() {
        val s = DeviceIconShapes
        assertEquals("M6 2H15A2 2 0 0 1 17 4V20A2 2 0 0 1 15 22H6A2 2 0 0 1 4 20V4A2 2 0 0 1 6 2Z", s.rr(4f, 2f, 13f, 20f, 2f))
        val ok = Regex("^[MLHVCSAZmlhvcsaz0-9 .,-]+$")
        for (icon in s.ICONS) {
            val shape = s.shape(icon)
            val all = shape.parts + (shape.mark?.let { s.markParts(it.kind) } ?: emptyList())
            for (part in all) assertTrue("$icon: ${part.d}", ok.matches(part.d) && part.d.startsWith("M"))
        }
    }

    @Test
    fun handheldIcons() {
        assertTrue(DeviceIconShapes.isHandheld("android_foldable"))
        assertTrue(DeviceIconShapes.isHandheld("phone"))
        assertFalse(DeviceIconShapes.isHandheld("mac_laptop"))
        assertFalse(DeviceIconShapes.isHandheld(null))
    }
}
