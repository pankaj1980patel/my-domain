package com.mydomain.android.core

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class KeyChangePlanTest {
    private fun change(node: String, dismissed: Boolean = false) =
        KeyChangeInfo(node, node.uppercase(), "AAAA BBBB CCCC DDDD", "1111 2222 3333 4444", dismissed = dismissed)

    @Test
    fun theScreenAsksOnlyAboutAChangeNotDismissed() {
        val list = listOf(change("phone", dismissed = true), change("tablet"))
        assertNull(KeyChangePlan.banner(list, "phone"))
        assertEquals("tablet", KeyChangePlan.banner(list, "tablet")?.nodeId)
        assertNull(KeyChangePlan.banner(list, "laptop"))
    }

    @Test
    fun theMapShowsItEvenWhenDismissed() {
        val list = listOf(change("phone", dismissed = true))
        assertEquals("phone", KeyChangePlan.banner(list, "phone", showDismissed = true)?.nodeId)
    }

    @Test
    fun oneNotificationPerDevice() {
        val a = KeyChangePlan.notificationId("phone")
        assertEquals(a, KeyChangePlan.notificationId("phone"))
        assertNotEquals(a, KeyChangePlan.notificationId("laptop"))
        assertTrue(a and 0x6000_0000 == 0x6000_0000)
    }

    @Test
    fun namesPreferTheLabelThenTheCoreThenTheId() {
        val c = change("phone")
        assertEquals("My Pixel", KeyChangePlan.nameOf(c) { "My Pixel" })
        assertEquals("PHONE", KeyChangePlan.nameOf(c) { null })
        assertEquals("phone", KeyChangePlan.nameOf(c.copy(name = "")) { " " })
    }
}
