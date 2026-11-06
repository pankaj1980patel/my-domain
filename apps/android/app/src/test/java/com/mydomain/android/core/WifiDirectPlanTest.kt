package com.mydomain.android.core

import com.mydomain.android.core.WifiDirectPlan.CAN_HOST
import com.mydomain.android.core.WifiDirectPlan.CAN_HOTSPOT
import com.mydomain.android.core.WifiDirectPlan.CAN_JOIN
import com.mydomain.android.core.WifiDirectPlan.CAN_JOIN_HOTSPOT
import com.mydomain.android.core.WifiDirectPlan.Event
import com.mydomain.android.core.WifiDirectPlan.Method
import com.mydomain.android.core.WifiDirectPlan.Phase
import com.mydomain.android.core.WifiDirectPlan.Role
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class WifiDirectPlanTest {
    private val both = CAN_HOST or CAN_JOIN

    @Test
    fun whatAPhoneCanDo() {
        assertEquals(0L, WifiDirectPlan.caps(33, hasWifiDirect = false))
        assertEquals(both, WifiDirectPlan.caps(29, hasWifiDirect = true))
        // Before Android 10 it can't join by name: host only.
        assertEquals(CAN_HOST, WifiDirectPlan.caps(28, hasWifiDirect = true))
        assertEquals(both or CAN_HOTSPOT, WifiDirectPlan.caps(30, true, hasHotspot = true))
        // A local-only hotspot needs Android 8.
        assertEquals(0L, WifiDirectPlan.caps(25, false, hasHotspot = true))
    }

    @Test
    fun whoHostsIsTheSameOnBothEnds() {
        assertEquals(Role.Host, WifiDirectPlan.chooseRole("a", both, "b", both))
        assertEquals(Role.Join, WifiDirectPlan.chooseRole("b", both, "a", both))
        // A Windows PC only hosts.
        assertEquals(Role.Join, WifiDirectPlan.chooseRole("z", both, "a", CAN_HOST))
        assertNull(WifiDirectPlan.chooseRole("a", CAN_HOST, "b", CAN_HOST))
        assertNull(WifiDirectPlan.chooseRole("a", both, "b", 0))
    }

    @Test
    fun wifiDirectFirstThenThePhonesHotspot() {
        val phone = both or CAN_HOTSPOT
        assertEquals(Method.WifiDirect to Role.Join, WifiDirectPlan.chooseMethod("p", phone, "w", CAN_HOST or CAN_JOIN_HOTSPOT))
        assertEquals(Method.Hotspot to Role.Host, WifiDirectPlan.chooseMethod("p", phone, "m", CAN_JOIN_HOTSPOT))
        assertNull(WifiDirectPlan.chooseMethod("p", both, "m", CAN_JOIN_HOTSPOT))
    }

    @Test
    fun onlyBigFiles() {
        assertTrue(WifiDirectPlan.worthIt(WifiDirectPlan.MIN_BYTES))
        assertFalse(WifiDirectPlan.worthIt(WifiDirectPlan.MIN_BYTES - 1))
        assertTrue(WifiDirectPlan.worthIt(5, minBytes = 5))
    }

    @Test
    fun whichPermissionAndWhenToAsk() {
        // This app targets 28: location, even on Android 13.
        assertEquals(listOf("android.permission.ACCESS_FINE_LOCATION"), WifiDirectPlan.permissionsFor(34, 28))
        assertEquals(listOf("android.permission.NEARBY_WIFI_DEVICES"), WifiDirectPlan.permissionsFor(33, 33))
        assertEquals(listOf("android.permission.ACCESS_FINE_LOCATION"), WifiDirectPlan.permissionsFor(29, 33))
        assertTrue(WifiDirectPlan.shouldAsk(granted = false, askedBefore = false))
        assertFalse("never twice", WifiDirectPlan.shouldAsk(granted = false, askedBefore = true))
        assertFalse(WifiDirectPlan.shouldAsk(granted = true, askedBefore = false))
    }

    @Test
    fun theRequestStateMachine() {
        var p = Phase.Idle
        p = WifiDirectPlan.next(p, Event.Host)!!
        assertEquals(Phase.Hosting, p)
        assertNull("a second host is ignored", WifiDirectPlan.next(p, Event.Host))
        p = WifiDirectPlan.next(p, Event.GroupUp)!!
        assertEquals(Phase.Hosted, p)
        assertNull("connected is for a joiner", WifiDirectPlan.next(p, Event.Connected))
        assertTrue(WifiDirectPlan.removeOnLeave(p))
        p = WifiDirectPlan.next(p, Event.Leave)!!
        assertEquals(Phase.Left, p)
        assertNull("a late success after leave", WifiDirectPlan.next(p, Event.GroupUp))

        var j = WifiDirectPlan.next(Phase.Idle, Event.Join)!!
        assertEquals(Phase.Joining, j)
        assertTrue("a join under way is removed too", WifiDirectPlan.removeOnLeave(j))
        j = WifiDirectPlan.next(j, Event.Failure)!!
        assertEquals(Phase.Failed, j)
        assertFalse(WifiDirectPlan.removeOnLeave(j))
        assertFalse(WifiDirectPlan.removeOnLeave(Phase.Idle))
    }

    @Test
    fun groupNamesAndTheHotspotsAddress() {
        assertTrue(WifiDirectPlan.validGroup("DIRECT-md-0f3a77bc", "0123456789abcdef"))
        assertFalse(WifiDirectPlan.validGroup("Home", "0123456789abcdef"))
        assertFalse(WifiDirectPlan.validGroup("DIRECT-md-x", "short"))
        val ifaces = listOf("wlan0" to "192.168.1.20", "swlan0" to "192.168.43.1", "lo" to "127.0.0.1")
        assertEquals("192.168.43.1", WifiDirectPlan.pickHotspotAddress(ifaces, "192.168.1.20"))
        assertNull(WifiDirectPlan.pickHotspotAddress(listOf("wlan0" to "192.168.1.20"), "192.168.1.20"))
        // Its own Wi-Fi dropped: still not the mobile data one.
        assertEquals("10.0.5.1", WifiDirectPlan.pickHotspotAddress(listOf("rmnet0" to "100.1.2.3", "ap0" to "10.0.5.1"), null))
    }
}
