package com.mydomain.android.core

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class NetWatchTest {
    private val wifi = NetWatch.Snapshot(network = 100, wifi = true, validated = true, wifiIp = "192.168.0.7")

    @Test
    fun kindOfNetwork() {
        assertEquals("offline", NetWatch.kind(null))
        assertEquals("offline", NetWatch.kind(NetWatch.Snapshot(network = null)))
        assertEquals("wifi", NetWatch.kind(wifi))
        assertEquals("mobile", NetWatch.kind(NetWatch.Snapshot(network = 7, cellular = true)))
        assertEquals("ethernet", NetWatch.kind(NetWatch.Snapshot(network = 7, ethernet = true)))
        // Wi-Fi wins over a cellular flag (a carrier Wi-Fi network).
        assertEquals("wifi", NetWatch.kind(NetWatch.Snapshot(network = 7, wifi = true, cellular = true)))
        assertEquals("unknown", NetWatch.kind(NetWatch.Snapshot(network = 7)))
    }

    @Test
    fun onlyRealChangesReachTheCore() {
        // The first look, and the same network again (a signal-strength
        // callback): only the first counts.
        assertTrue(NetWatch.significant(null, wifi))
        assertFalse(NetWatch.significant(wifi, wifi.copy()))
        // Another Wi-Fi network (a switch), Wi-Fi to mobile data, internet
        // validated, a new address, no network at all.
        assertTrue(NetWatch.significant(wifi, wifi.copy(network = 101)))
        assertTrue(NetWatch.significant(wifi, NetWatch.Snapshot(network = 200, cellular = true, validated = true)))
        assertTrue(NetWatch.significant(wifi.copy(validated = false), wifi))
        assertTrue(NetWatch.significant(wifi, wifi.copy(wifiIp = "192.168.0.8")))
        assertTrue(NetWatch.significant(wifi, NetWatch.Snapshot(network = null)))
    }

    @Test
    fun backoffDoublesToTheCapWithTwentyPercentJitter() {
        val mid = { a: Int -> NetWatch.backoffMs(a, 1_000, 30_000, 0.5) }
        assertEquals(1_000L, mid(0))
        assertEquals(2_000L, mid(1))
        assertEquals(16_000L, mid(4))
        assertEquals(30_000L, mid(5))
        assertEquals(30_000L, mid(50))
        assertEquals(800L, NetWatch.backoffMs(0, 1_000, 30_000, 0.0))
        assertEquals(1_200L, NetWatch.backoffMs(0, 1_000, 30_000, 1.0))
        assertEquals(1_200L, NetWatch.backoffMs(0, 1_000, 30_000, 9.0))
        assertEquals(800L, NetWatch.backoffMs(-3, 1_000, 30_000, 0.0))
    }

    @Test
    fun healthCheckComesSoonerAfterTheServiceWasKilled() {
        assertEquals(NetWatch.CHECK_EVERY_MS, NetWatch.nextCheckMs(0, 0.5))
        assertEquals(NetWatch.CHECK_AGAIN_MS, NetWatch.nextCheckMs(1, 0.5))
        assertEquals(2 * NetWatch.CHECK_AGAIN_MS, NetWatch.nextCheckMs(2, 0.5))
        assertEquals(NetWatch.CHECK_EVERY_MS, NetWatch.nextCheckMs(20, 0.5))
    }
}

class OemAutostartTest {
    @Test
    fun makersByManufacturerOrBrand() {
        assertEquals(OemAutostart.Maker.VIVO, OemAutostart.maker("vivo", "iQOO"))
        assertEquals(OemAutostart.Maker.VIVO, OemAutostart.maker("", "iqoo"))
        assertEquals(OemAutostart.Maker.XIAOMI, OemAutostart.maker("Xiaomi", "Redmi"))
        assertEquals(OemAutostart.Maker.XIAOMI, OemAutostart.maker("POCO"))
        assertEquals(OemAutostart.Maker.OPPO, OemAutostart.maker("realme"))
        assertEquals(OemAutostart.Maker.OPPO, OemAutostart.maker("OPPO"))
        assertEquals(OemAutostart.Maker.ONEPLUS, OemAutostart.maker("OnePlus"))
        assertEquals(OemAutostart.Maker.SAMSUNG, OemAutostart.maker("samsung"))
        assertEquals(OemAutostart.Maker.HUAWEI, OemAutostart.maker("HONOR"))
        assertNull(OemAutostart.maker("Google", "google"))
        assertFalse(OemAutostart.known("Google", "google"))
        assertTrue(OemAutostart.known("vivo"))
    }

    @Test
    fun pagesMostSpecificFirst() {
        // The user's phone: vivo / iQOO ("I2405").
        val vivo = OemAutostart.pages("vivo", "iQOO")
        assertEquals("com.vivo.permissionmanager", vivo.first().pkg)
        assertTrue(vivo.any { it.pkg == "com.iqoo.secure" })
        assertTrue(vivo.any { it.pkg == "com.vivo.abe" })
        assertEquals(
            "com.miui.permcenter.autostart.AutoStartManagementActivity",
            OemAutostart.pages("Xiaomi").first().cls,
        )
        // OnePlus: its own page, then ColorOS's (newer OnePlus phones).
        val op = OemAutostart.pages("OnePlus")
        assertEquals("com.oneplus.security", op.first().pkg)
        assertTrue(op.containsAll(OemAutostart.pages("oppo")))
        assertEquals(OemAutostart.pages("oppo"), OemAutostart.pages("realme"))
        assertTrue(OemAutostart.pages("samsung").all { it.pkg.startsWith("com.samsung.android.") })
        assertTrue(OemAutostart.pages("Google").isEmpty())
        // Every page is a full class name.
        for (m in listOf("xiaomi", "oppo", "oneplus", "vivo", "samsung", "huawei")) {
            for (p in OemAutostart.pages(m)) assertTrue(p.cls, p.cls.contains('.'))
        }
    }
}
