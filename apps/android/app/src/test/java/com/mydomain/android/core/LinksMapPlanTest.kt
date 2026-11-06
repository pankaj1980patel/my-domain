package com.mydomain.android.core

import com.mydomain.android.core.LinksMapPlan.Pt
import com.mydomain.android.core.LinksMapPlan.Tone
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import kotlin.math.sqrt

class LinksMapPlanTest {
    private val w = 600f
    private val h = 420f

    private fun dist(a: Pt, b: Pt) = sqrt((a.x - b.x) * (a.x - b.x) + (a.y - b.y) * (a.y - b.y))

    @Test
    fun devicesSitAroundTheMiddle() {
        assertTrue(LinksMapPlan.ring(0, w, h).isEmpty())
        val one = LinksMapPlan.ring(1, w, h)
        assertEquals(w / 2, one[0].x, 0.1f)
        assertTrue("the first at the top", one[0].y < h / 2)
        val two = LinksMapPlan.ring(2, w, h)
        assertTrue("left and right", two[0].x < w / 2 && two[1].x > w / 2)
        assertEquals(two[0].y, two[1].y, 0.1f)
        for (n in 1..12) {
            val pts = LinksMapPlan.ring(n, w, h)
            assertEquals(n, pts.size)
            assertTrue(pts.all { it.x in 0f..w && it.y in 0f..h })
            for (i in pts.indices) for (j in i + 1 until pts.size) {
                assertTrue("$n: overlap", dist(pts[i], pts[j]) > 40f)
            }
        }
    }

    @Test
    fun linesAreDrawnByHow() {
        assertEquals(Tone.LAN, LinksMapPlan.tone("lan"))
        assertEquals(Tone.LAN, LinksMapPlan.tone("wifi_direct"))
        assertEquals(Tone.LAN, LinksMapPlan.tone("hotspot"))
        assertEquals(Tone.DIRECT, LinksMapPlan.tone("punched"))
        assertEquals(Tone.DIRECT, LinksMapPlan.tone("webrtc"))
        assertEquals(Tone.RELAY, LinksMapPlan.tone("relay"))
        assertEquals(Tone.BLE, LinksMapPlan.tone("ble"))
        assertEquals(Tone.NONE, LinksMapPlan.tone(null))
        assertNull(LinksMapPlan.dash("lan"))
        assertNull(LinksMapPlan.dash("relay"))
        assertEquals(6f, LinksMapPlan.dash(null)!![0])
        assertEquals(2f, LinksMapPlan.dash("ble")!![0])
        assertNull(LinksMapPlan.latency(null))
        assertEquals("<1 ms", LinksMapPlan.latency(0))
        assertEquals("42 ms", LinksMapPlan.latency(42))
    }

    @Test
    fun otherLinesBowAwayFromTheMiddle() {
        val c = Pt(w / 2, h / 2)
        // Opposite each other: straight through the middle unless bowed.
        val (_, label) = LinksMapPlan.bow(Pt(100f, h / 2), Pt(500f, h / 2), c)
        assertTrue(dist(label, c) > 30f)
        val a = Pt(300f, 60f)
        val b = Pt(520f, 210f)
        val (_, l2) = LinksMapPlan.bow(a, b, c)
        val mid = Pt((a.x + b.x) / 2, (a.y + b.y) / 2)
        assertTrue("further out than the chord", dist(l2, c) > dist(mid, c))
    }

    @Test
    fun reasonsAreMappedAndShownUnlessDirect() {
        assertEquals("lan_blocked", LinksMapPlan.code("lan", "blocked"))
        assertEquals("internet_symmetric_nat", LinksMapPlan.code("internet", "symmetric_nat"))
        assertEquals("relay_auth_failed", LinksMapPlan.code("relay", "auth_failed"))
        assertEquals("ble_off", LinksMapPlan.code("ble", "off"))
        assertEquals("device_bad_key", LinksMapPlan.code("device", "bad_key"))
        assertEquals("unknown", LinksMapPlan.code("lan", "gremlins"))
        assertEquals("unknown", LinksMapPlan.code("pigeon", "lost"))

        val tries = listOf(PathTryInfo("lan", "other_network", 9), PathTryInfo("relay", "peer_offline", 8))
        val down = MapDeviceInfo("p", "Phone", null, tries = tries, problem = "bad_key")
        val w = LinksMapPlan.whys(down)
        assertEquals(listOf("lan_other_network", "relay_peer_offline", "device_bad_key"), w.map { it.code })
        assertEquals(9L, w[0].agoS)
        assertNull(w[2].agoS)

        val relayed = down.copy(link = MapLinkInfo("relay", "relay"), problem = null)
        assertEquals(2, LinksMapPlan.whys(relayed).size)
        val direct = down.copy(link = MapLinkInfo("lan", "quic"))
        assertTrue(LinksMapPlan.whys(direct).isEmpty())
    }

    @Test
    fun onlyLinksBetweenShownDevicesAreDrawn() {
        val map = LinksMapInfo(
            "me", "Phone",
            listOf(MapDeviceInfo("mac", "Mac", null), MapDeviceInfo("pc", "PC", null)),
            listOf(
                OtherLinkInfo("mac", "pc", "lan", 3, "mac", 1),
                OtherLinkInfo("ghost", "pc", "lan", null, "pc", 1),
            ),
        )
        assertEquals(listOf("mac"), LinksMapPlan.drawable(map).map { it.a })
    }
}
