package com.mydomain.android.core

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class AutomationRulesTest {

    private fun auto(id: String, trigger: Trigger, action: String = "lock", node: String = "mac") =
        Automation(id = id, trigger = trigger, node = node, action = action, args = "{}", label = "Lock it")

    // --- links -----------------------------------------------------------------

    @Test
    fun connectsOnFirstUpAndAfterAway() {
        val b = AutomationBook()
        assertEquals(Edge.Connected("mac"), b.link("mac", true, 0))
        assertNull(b.link("mac", true, 1_000))
        // Down: nothing until a minute has passed.
        assertNull(b.link("mac", false, 2_000))
        assertNull(b.link("mac", false, 61_999))
        assertEquals(Edge.Disconnected("mac"), b.link("mac", false, 62_000))
        // Only once.
        assertNull(b.link("mac", false, 200_000))
        // Back: connects again.
        assertEquals(Edge.Connected("mac"), b.link("mac", true, 300_000))
    }

    @Test
    fun blipIsNeitherDisconnectNorConnect() {
        val b = AutomationBook()
        b.link("mac", true, 0)
        assertNull(b.link("mac", false, 10_000))
        assertNull(b.link("mac", true, 40_000))
        assertNull(b.link("mac", false, 50_000))
        // The minute counts from the latest drop.
        assertNull(b.link("mac", false, 100_000))
        assertEquals(Edge.Disconnected("mac"), b.link("mac", false, 110_000))
    }

    @Test
    fun neverSeenUpNeverDisconnects() {
        val b = AutomationBook()
        assertTrue(b.links(listOf("mac", "pc"), emptySet(), 0).isEmpty())
        assertTrue(b.links(listOf("mac", "pc"), emptySet(), 600_000).isEmpty())
        assertEquals(listOf<Edge>(Edge.Connected("pc")), b.links(listOf("mac", "pc"), setOf("pc"), 700_000))
    }

    // --- battery ---------------------------------------------------------------

    @Test
    fun firstBatteryOnlySetsBaseline() {
        val b = AutomationBook()
        assertTrue(b.battery(50, false).isEmpty())
        assertEquals(listOf<Edge>(Edge.Battery(50, 49)), b.battery(49, false))
        assertTrue(b.battery(49, false).isEmpty())
        assertEquals(listOf(Edge.ChargingStarted), b.battery(49, true))
        assertEquals(listOf(Edge.Battery(49, 50), Edge.ChargingStopped), b.battery(50, false))
    }

    @Test
    fun batteryBelowFiresOnCrossingOnly() {
        val t = Trigger(TriggerKind.BATTERY_BELOW, level = 20)
        assertTrue(AutomationBook.matches(t, Edge.Battery(20, 19)))
        assertTrue(AutomationBook.matches(t, Edge.Battery(25, 10)))
        assertFalse(AutomationBook.matches(t, Edge.Battery(19, 18)))
        assertFalse(AutomationBook.matches(t, Edge.Battery(21, 20)))
        assertFalse(AutomationBook.matches(t, Edge.Battery(18, 21)))
    }

    // --- Wi-Fi -----------------------------------------------------------------

    @Test
    fun wifiJoinAndLeave() {
        val b = AutomationBook()
        assertTrue(b.wifi(false, null).isEmpty())
        assertEquals(listOf<Edge>(Edge.WifiJoined("Home")), b.wifi(true, "Home"))
        assertTrue(b.wifi(true, "Home").isEmpty())
        // Roaming to another named network: leave, then join.
        assertEquals(listOf(Edge.WifiLeft("Home"), Edge.WifiJoined("Office")), b.wifi(true, "Office"))
        assertEquals(listOf<Edge>(Edge.WifiLeft("Office")), b.wifi(false, null))
    }

    @Test
    fun wifiBaselineConnected() {
        val b = AutomationBook()
        assertTrue(b.wifi(true, "Home").isEmpty())
        assertEquals(listOf<Edge>(Edge.WifiLeft("Home")), b.wifi(false, null))
    }

    @Test
    fun wifiNameArrivingLate() {
        val b = AutomationBook()
        b.wifi(false, null)
        assertEquals(listOf<Edge>(Edge.WifiJoined(null)), b.wifi(true, null))
        assertEquals(listOf<Edge>(Edge.WifiJoined("Home", late = true)), b.wifi(true, "Home"))
        // Not again.
        assertTrue(b.wifi(true, "Home").isEmpty())
        // The name going unknown again isn't a change.
        assertTrue(b.wifi(true, null).isEmpty())
        assertEquals(listOf<Edge>(Edge.WifiLeft("Home")), b.wifi(false, null))
    }

    @Test
    fun wifiNameLearntAtStartIsNotAJoin() {
        val b = AutomationBook()
        assertTrue(b.wifi(true, null).isEmpty())
        assertTrue(b.wifi(true, "Home").isEmpty())
    }

    @Test
    fun wifiMatching() {
        val any = Trigger(TriggerKind.WIFI_JOINS)
        val home = Trigger(TriggerKind.WIFI_JOINS, ssid = "Home")
        assertTrue(AutomationBook.matches(any, Edge.WifiJoined(null)))
        assertTrue(AutomationBook.matches(any, Edge.WifiJoined("Home")))
        assertFalse(AutomationBook.matches(any, Edge.WifiJoined("Home", late = true)))
        assertTrue(AutomationBook.matches(home, Edge.WifiJoined("home")))
        assertTrue(AutomationBook.matches(home, Edge.WifiJoined("Home", late = true)))
        assertFalse(AutomationBook.matches(home, Edge.WifiJoined(null)))
        assertFalse(AutomationBook.matches(home, Edge.WifiJoined("Office")))
        assertFalse(AutomationBook.matches(home, Edge.WifiLeft("Home")))
        val leaveAny = Trigger(TriggerKind.WIFI_LEAVES)
        val leaveHome = Trigger(TriggerKind.WIFI_LEAVES, ssid = "Home")
        assertTrue(AutomationBook.matches(leaveAny, Edge.WifiLeft(null)))
        assertTrue(AutomationBook.matches(leaveHome, Edge.WifiLeft("Home")))
        assertFalse(AutomationBook.matches(leaveHome, Edge.WifiLeft(null)))
    }

    // --- matching and the rate limit -------------------------------------------

    @Test
    fun matchesTheWatchedDeviceOnly() {
        val t = Trigger(TriggerKind.CONNECTS, node = "mac")
        assertTrue(AutomationBook.matches(t, Edge.Connected("mac")))
        assertFalse(AutomationBook.matches(t, Edge.Connected("pc")))
        assertFalse(AutomationBook.matches(t, Edge.Disconnected("mac")))
        assertTrue(AutomationBook.matches(Trigger(TriggerKind.CHARGING_STARTS), Edge.ChargingStarted))
        assertFalse(AutomationBook.matches(Trigger(TriggerKind.CHARGING_STARTS), Edge.ChargingStopped))
    }

    @Test
    fun atMostOnceAMinute() {
        val b = AutomationBook()
        b.add(auto("a", Trigger(TriggerKind.CONNECTS, node = "mac")))
        b.add(auto("b", Trigger(TriggerKind.CHARGING_STARTS)))
        val first = b.take(Edge.Connected("mac"), 1_000)
        assertEquals(listOf("a"), first.map { it.id })
        assertEquals(1_000L, b.list.first { it.id == "a" }.lastFired)
        assertTrue(b.take(Edge.Connected("mac"), 30_000).isEmpty())
        assertEquals(listOf("a"), b.take(Edge.Connected("mac"), 61_000).map { it.id })
        assertNull(b.list.first { it.id == "b" }.lastFired)
    }

    @Test
    fun disabledNeverRuns() {
        val b = AutomationBook()
        b.add(auto("a", Trigger(TriggerKind.CHARGING_STOPS)))
        b.setEnabled("a", false)
        assertTrue(b.take(Edge.ChargingStopped, 0).isEmpty())
        b.setEnabled("a", true)
        assertEquals(1, b.take(Edge.ChargingStopped, 0).size)
    }

    @Test
    fun cooledAllowsAClockThatWentBack() {
        assertTrue(AutomationBook.cooled(null, 0))
        assertFalse(AutomationBook.cooled(100_000, 120_000))
        assertTrue(AutomationBook.cooled(100_000, 160_000))
        assertTrue(AutomationBook.cooled(100_000, 50_000))
    }

    // --- checking --------------------------------------------------------------

    @Test
    fun checkedTidiesAndRefuses() {
        val ok = AutomationBook.checked(
            Automation("x", Trigger(TriggerKind.CHARGING_STARTS, node = "mac", level = 5, ssid = "Home"), " mac ", "lock", "", "  Lock it  "),
        )
        assertEquals(Trigger(TriggerKind.CHARGING_STARTS), ok.trigger)
        assertEquals("mac", ok.node)
        assertEquals("{}", ok.args)
        assertEquals("Lock it", ok.label)

        fun refused(a: Automation) = runCatching { AutomationBook.checked(a) }.isFailure
        assertTrue(refused(auto("x", Trigger("explodes"))))
        assertTrue(refused(auto("x", Trigger(TriggerKind.CONNECTS))))
        assertTrue(refused(auto("x", Trigger(TriggerKind.BATTERY_BELOW, level = 0))))
        assertTrue(refused(auto("x", Trigger(TriggerKind.BATTERY_BELOW, level = 100))))
        assertTrue(refused(auto("x", Trigger(TriggerKind.BATTERY_BELOW))))
        assertTrue(refused(auto("x", Trigger(TriggerKind.WIFI_JOINS, ssid = "x".repeat(33)))))
        assertTrue(refused(auto("x", Trigger(TriggerKind.CHARGING_STARTS), action = "")))
        assertTrue(refused(auto("x", Trigger(TriggerKind.CHARGING_STARTS)).copy(args = "[1]")))
        assertFalse(refused(auto("x", Trigger(TriggerKind.BATTERY_BELOW, level = 20))))
    }

    @Test
    fun updateKeepsLastFired() {
        val b = AutomationBook()
        b.add(auto("a", Trigger(TriggerKind.CHARGING_STARTS)))
        b.take(Edge.ChargingStarted, 5_000)
        b.update(auto("a", Trigger(TriggerKind.CHARGING_STOPS)))
        assertEquals(TriggerKind.CHARGING_STOPS, b.list.single().trigger.kind)
        assertEquals(5_000L, b.list.single().lastFired)
        assertTrue(b.delete("a"))
        assertFalse(b.delete("a"))
    }

    // --- saving ----------------------------------------------------------------

    @Test
    fun roundTrip() {
        val list = listOf(
            Automation(
                "1", Trigger(TriggerKind.CONNECTS, node = "mac"), "", "ringer", """{"mode":"silent"}""",
                "Set the ringer to \"silent\"\n", true, 1_700_000_000_000,
            ),
            Automation("2", Trigger(TriggerKind.BATTERY_BELOW, level = 15), "mac", "notify", "{}", "Tell me", false, null),
            Automation("3", Trigger(TriggerKind.WIFI_LEAVES, ssid = "Café ☕"), "mac", "send_text", """{"text":"bye\\u0001"}""", "Send a note"),
        )
        val back = AutomationBook.decode(AutomationBook.encode(list))
        // Labels are trimmed on the way in.
        assertEquals(list.map { it.copy(label = it.label.trim()) }, back)
    }

    @Test
    fun decodeSkipsBadEntries() {
        val text = """
            {"automations": [
              {"id": "ok", "trigger": {"kind": "charging_starts"}, "node": "", "action": "notify"},
              {"id": "bad", "trigger": {"kind": "teleports"}, "node": "", "action": "notify"},
              {"id": "ok", "trigger": {"kind": "charging_stops"}, "node": "", "action": "notify"},
              "nonsense", 42
            ], "future": {"x": [1, 2.5, -3e2, null, true]}}
        """
        val list = AutomationBook.decode(text)
        assertEquals(1, list.size)
        assertEquals(TriggerKind.CHARGING_STARTS, list[0].trigger.kind)
        assertTrue(list[0].enabled)
        assertTrue(AutomationBook.decode("not json").isEmpty())
        assertTrue(AutomationBook.decode("").isEmpty())
    }

    @Test
    fun jsonReadsAndWrites() {
        val v = Json.parse("""{"a":[1,2.5,"x\"y\u00e9",true,false,null],"b":{}}""")
        assertEquals(mapOf("a" to listOf(1L, 2.5, "x\"yé", true, false, null), "b" to emptyMap<String, Any?>()), v)
        assertEquals("""{"s":"a\nb\u0001","n":3}""", Json.write(linkedMapOf("s" to "a\nb\u0001", "n" to 3)))
        assertTrue(runCatching { Json.parse("{\"a\":1} x") }.isFailure)
        assertTrue(runCatching { Json.parse("[1,") }.isFailure)
    }
}
