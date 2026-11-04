package com.mydomain.android.core

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Test

class AutomationAppsTest {
    private fun extras(vararg kv: Pair<String, String>): (String) -> String? = { k -> kv.toMap()[k] }

    private fun refused(block: () -> Unit): String {
        try {
            block()
        } catch (e: AppRefusal) {
            return e.message ?: ""
        }
        fail("expected a refusal")
        return ""
    }

    private val mac = AppPeer("n-mac", "Work Mac", listOf("Work Mac", "MacBook-Pro.local", "MacBook Pro"))
    private val linux = AppPeer("n-linux", "pankaj laptop", listOf("pankaj-laptop", "pankaj laptop"))
    private val twin = AppPeer("n-twin", "MacBook Pro", listOf("MacBook-Pro", "MacBook Pro"))

    // --- parsing ---------------------------------------------------------------------

    @Test
    fun parsesRun() {
        val r = AutomationApps.parse(
            AutomationApps.ACTION_RUN,
            extras("device" to " Work Mac ", "action" to "mute", "args" to """{"on":true}"""),
        )
        assertEquals(AppRequest.Run("Work Mac", "mute", """{"on":true}"""), r)
        val bare = AutomationApps.parse(AutomationApps.ACTION_RUN, extras("device" to "x", "action" to "lock"))
        assertEquals(AppRequest.Run("x", "lock", "{}"), bare)
    }

    @Test
    fun refusesBadRuns() {
        refused { AutomationApps.parse(AutomationApps.ACTION_RUN, extras("action" to "lock")) }
        refused { AutomationApps.parse(AutomationApps.ACTION_RUN, extras("device" to "x")) }
        refused { AutomationApps.parse(AutomationApps.ACTION_RUN, extras("device" to "x", "action" to "rm -rf")) }
        refused { AutomationApps.parse(AutomationApps.ACTION_RUN, extras("device" to "x", "action" to "Lock")) }
        refused { AutomationApps.parse(AutomationApps.ACTION_RUN, extras("device" to "x", "action" to "lock", "args" to "[1]")) }
        refused { AutomationApps.parse(AutomationApps.ACTION_RUN, extras("device" to "x", "action" to "lock", "args" to "{bad")) }
        refused {
            AutomationApps.parse(
                AutomationApps.ACTION_RUN,
                extras("device" to "x", "action" to "lock", "args" to "{\"a\":\"" + "x".repeat(5000) + "\"}"),
            )
        }
        refused { AutomationApps.parse(AutomationApps.ACTION_RUN, extras("device" to "x".repeat(201), "action" to "lock")) }
        refused { AutomationApps.parse("com.mydomain.android.action.FORMAT_DISK", extras()) }
        refused { AutomationApps.parse(null, extras()) }
    }

    @Test
    fun parsesSendText() {
        assertEquals(
            AppRequest.SendText("mac", "hello", "clipboard"),
            AutomationApps.parse(AutomationApps.ACTION_SEND_TEXT, extras("device" to "mac", "text" to "hello")),
        )
        assertEquals(
            AppRequest.SendText("mac", "hi", "notify"),
            AutomationApps.parse(AutomationApps.ACTION_SEND_TEXT, extras("device" to "mac", "text" to "hi", "as" to "Notice")),
        )
        refused { AutomationApps.parse(AutomationApps.ACTION_SEND_TEXT, extras("device" to "mac", "text" to "  ")) }
        refused { AutomationApps.parse(AutomationApps.ACTION_SEND_TEXT, extras("device" to "mac", "text" to "hi", "as" to "shell")) }
        refused {
            AutomationApps.parse(AutomationApps.ACTION_SEND_TEXT, extras("device" to "mac", "text" to "x".repeat(AutomationApps.MAX_TEXT + 1)))
        }
    }

    @Test
    fun parsesAutomationAndList() {
        assertEquals(
            AppRequest.RunAutomation("Silence"),
            AutomationApps.parse(AutomationApps.ACTION_RUN_AUTOMATION, extras("automation" to "Silence")),
        )
        refused { AutomationApps.parse(AutomationApps.ACTION_RUN_AUTOMATION, extras()) }
        assertEquals(AppRequest.ListDevices, AutomationApps.parse(AutomationApps.ACTION_LIST_DEVICES, extras()))
    }

    // --- actions ------------------------------------------------------------------------

    @Test
    fun actionsFollowWhatTheDeviceSaid() {
        // Ring always; else only what it listed.
        AutomationApps.checkAction("ring", setOf("lock"), "Mac")
        AutomationApps.checkAction("lock", setOf("lock"), "Mac")
        refused { AutomationApps.checkAction("mute", setOf("lock"), "Mac") }
        refused { AutomationApps.checkAction("run", setOf("lock"), "Mac") }
        // Nothing said yet: the presets only.
        AutomationApps.checkAction("lock", null, "Mac")
        AutomationApps.checkAction("mute", emptySet(), "Mac")
        refused { AutomationApps.checkAction("run", null, "Mac") }
        refused { AutomationApps.checkAction("screen", null, "Mac") }
        // Text has its own request.
        refused { AutomationApps.checkAction("send_text", setOf("send_text"), "Mac") }
    }

    // --- devices --------------------------------------------------------------------------

    @Test
    fun findsDevicesByIdOrName() {
        val peers = listOf(mac, linux)
        assertEquals("n-mac", AutomationApps.findDevice(peers, "n-mac").node)
        assertEquals("n-mac", AutomationApps.findDevice(peers, "work mac").node)
        assertEquals("n-mac", AutomationApps.findDevice(peers, "MACBOOK-PRO.LOCAL").node)
        assertEquals("n-linux", AutomationApps.findDevice(peers, " Pankaj Laptop ").node)
        assertTrue(refused { AutomationApps.findDevice(peers, "toaster") }.contains("no device"))
        refused { AutomationApps.findDevice(peers, " ") }
    }

    @Test
    fun ambiguousNamesAreErrors() {
        val peers = listOf(mac, twin)
        assertTrue(refused { AutomationApps.findDevice(peers, "macbook pro") }.contains("more than one"))
        // The node id still works, and a unique nickname.
        assertEquals("n-twin", AutomationApps.findDevice(peers, "n-twin").node)
        assertEquals("n-mac", AutomationApps.findDevice(peers, "work mac").node)
    }

    @Test
    fun devicesJsonListsRingFirst() {
        val json = AutomationApps.devicesJson(listOf(mac, linux), setOf("n-mac")) { node ->
            if (node == "n-mac") setOf("mute", "lock", "ring") else null
        }
        assertEquals(
            """[{"name":"Work Mac","node":"n-mac","connected":true,"actions":["ring","lock","mute"]},""" +
                """{"name":"pankaj laptop","node":"n-linux","connected":false,"actions":["ring"]}]""",
            json,
        )
    }

    // --- automations ------------------------------------------------------------------------

    @Test
    fun findsAutomations() {
        val t = Trigger(TriggerKind.CHARGING_STARTS)
        val a = Automation("a1", t, "", "ringer", label = "Silence")
        val b = Automation("b2", t, "mac", "lock", label = "Lock it", enabled = false)
        val c = Automation("c3", t, "mac", "lock", label = "twice")
        val d = Automation("d4", t, "mac", "lock", label = "Twice")
        val list = listOf(a, b, c, d)
        assertEquals("a1", AutomationApps.findAutomation(list, "a1").id)
        assertEquals("a1", AutomationApps.findAutomation(list, "silence").id)
        assertTrue(refused { AutomationApps.findAutomation(list, "Lock it") }.contains("switched off"))
        assertTrue(refused { AutomationApps.findAutomation(list, "TWICE") }.contains("more than one"))
        assertEquals("d4", AutomationApps.findAutomation(list, "d4").id)
        refused { AutomationApps.findAutomation(list, "nope") }
    }

    // --- the key ------------------------------------------------------------------------------

    @Test
    fun keyFromExtraOrData() {
        assertEquals("abc", AutomationApps.keyOf(" abc ", null))
        assertEquals("abc", AutomationApps.keyOf(null, "key:abc"))
        assertEquals("abc", AutomationApps.keyOf("", "KEY: abc"))
        assertEquals(null, AutomationApps.keyOf(null, "https://example.com"))
        assertEquals(null, AutomationApps.keyOf(null, "key:"))
    }

    @Test
    fun keysMatchExactly() {
        assertTrue(AutomationApps.keyMatches("s3cret-key", "s3cret-key"))
        assertFalse(AutomationApps.keyMatches("s3cret-key", "s3cret-kez"))
        assertFalse(AutomationApps.keyMatches("s3cret-key", "s3cret"))
        assertFalse(AutomationApps.keyMatches("s3cret-key", "s3cret-key-longer"))
        assertFalse(AutomationApps.keyMatches("s3cret-key", null))
        assertFalse(AutomationApps.keyMatches("s3cret-key", ""))
        assertFalse(AutomationApps.keyMatches(null, "anything"))
        assertFalse(AutomationApps.keyMatches("", ""))
    }

    // --- rate limits --------------------------------------------------------------------------

    @Test
    fun windowLimitsPerMinute() {
        val w = RateWindow(6, 60_000)
        repeat(6) { assertTrue(w.hit(1_000L + it)) }
        assertFalse(w.hit(30_000))
        assertTrue(w.full(59_000))
        // A minute after the first: one more.
        assertTrue(w.hit(61_000))
        assertTrue(w.full(61_000))
        // Much later: all free.
        assertTrue(w.hit(500_000))
    }

    @Test
    fun windowSurvivesClockGoingBack() {
        val w = RateWindow(2, 60_000)
        assertTrue(w.hit(100_000))
        assertTrue(w.hit(100_001))
        assertTrue(w.hit(10_000))
    }

    // --- history ------------------------------------------------------------------------------

    @Test
    fun historyKeepsNewestFifty() {
        var list = emptyList<AppRun>()
        repeat(60) { list = AutomationApps.record(list, AppRun(it.toLong(), "net.dinglisch.android.taskerm", "ring $it", true, "quic")) }
        assertEquals(AutomationApps.HISTORY, list.size)
        assertEquals(59L, list.first().ts)
        val back = AutomationApps.decodeHistory(AutomationApps.encodeHistory(list))
        assertEquals(list, back)
        assertEquals(emptyList<AppRun>(), AutomationApps.decodeHistory("not json"))
        assertEquals(emptyList<AppRun>(), AutomationApps.decodeHistory(null))
        assertEquals(1, AutomationApps.decodeHistory("""[{"ts":1,"what":"x"},{"nope":1}]""").size)
    }

    @Test
    fun resultJson() {
        assertEquals("""{"ok":true,"via":"quic"}""", AutomationApps.resultJson(Result.success("quic")))
        assertEquals(
            """{"ok":false,"error":"no \"x\""}""",
            AutomationApps.resultJson(Result.failure(AppRefusal("no \"x\""))),
        )
    }
}
