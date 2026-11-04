package com.mydomain.android.core

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class SetupChecklistTest {

    private fun statusOf(rows: List<SetupRow>, item: SetupItem) = rows.first { it.item == item }.status

    @Test
    fun oldPhoneSkipsNewerItems() {
        val c = SetupChecklist(sdkInt = 28, targetSdk = 28)
        assertFalse(c.applies(SetupItem.FULL_SCREEN))
        assertFalse(c.applies(SetupItem.NEARBY_DEVICES))
        assertTrue(c.applies(SetupItem.WRITE_SETTINGS))
        assertTrue(c.applies(SetupItem.PHOTOS))
    }

    @Test
    fun runtimePermissionNeedsTheTargetToo() {
        // Android 14 phone, app targeting 28: Bluetooth permission never asked.
        assertFalse(SetupChecklist(34, 28).applies(SetupItem.NEARBY_DEVICES))
        assertTrue(SetupChecklist(34, 34).applies(SetupItem.NEARBY_DEVICES))
        assertFalse(SetupChecklist(30, 34).applies(SetupItem.NEARBY_DEVICES))
        // Full-screen intent depends on the phone only.
        assertTrue(SetupChecklist(34, 28).applies(SetupItem.FULL_SCREEN))
        assertFalse(SetupChecklist(33, 34).applies(SetupItem.FULL_SCREEN))
    }

    @Test
    fun statusesAndCount() {
        val on = setOf(SetupItem.NOTIFICATIONS, SetupItem.PHOTOS, SetupItem.NEARBY_DEVICES)
        val asked = mutableListOf<SetupItem>()
        val rows = SetupChecklist(30, 28).rows { asked += it; it in on }
        assertEquals(SetupStatus.ON, statusOf(rows, SetupItem.NOTIFICATIONS))
        assertEquals(SetupStatus.OFF, statusOf(rows, SetupItem.BATTERY))
        assertEquals(SetupStatus.NOT_NEEDED, statusOf(rows, SetupItem.NEARBY_DEVICES))
        assertEquals(SetupStatus.NOT_NEEDED, statusOf(rows, SetupItem.FULL_SCREEN))
        assertEquals(SetupStatus.INFO, statusOf(rows, SetupItem.QUICK_TILES))
        // Not-needed and explain-only items are never read.
        assertFalse(SetupItem.NEARBY_DEVICES in asked)
        assertFalse(SetupItem.QUICK_TILES in asked)
        // 13 items: 2 not needed, 1 explain-only -> 10 counted, 2 on.
        assertEquals(2 to 10, SetupChecklist.count(rows))
    }

    @Test
    fun orderIsStableThenInfoThenNotNeeded() {
        val c = SetupChecklist(30, 28)
        val a = c.rows { false }.map { it.item }
        val b = c.rows { true }.map { it.item }
        assertEquals(a, b)
        assertEquals(SetupItem.NOTIFICATIONS, a.first())
        val rows = c.rows { it == SetupItem.WIDGET }
        val statuses = rows.map { it.status }
        val firstInfo = statuses.indexOf(SetupStatus.INFO)
        val firstNotNeeded = statuses.indexOf(SetupStatus.NOT_NEEDED)
        assertTrue(statuses.take(firstInfo).all { it == SetupStatus.ON || it == SetupStatus.OFF })
        assertTrue(firstInfo < firstNotNeeded)
        assertTrue(statuses.drop(firstNotNeeded).all { it == SetupStatus.NOT_NEEDED })
        // Applicable items keep the enum's order.
        val applicable = rows.filter { it.status == SetupStatus.ON || it.status == SetupStatus.OFF }.map { it.item }
        assertEquals(applicable.sortedBy { it.ordinal }, applicable)
    }

    @Test
    fun everythingOnCountsAll() {
        val rows = SetupChecklist(35, 35).rows { true }
        assertEquals(12 to 12, SetupChecklist.count(rows))
    }

    @Test
    fun homeCard() {
        assertTrue(SetupChecklist.showCard(signedIn = true, dismissed = false, on = 3, total = 10))
        assertFalse(SetupChecklist.showCard(signedIn = false, dismissed = false, on = 3, total = 10))
        assertFalse(SetupChecklist.showCard(signedIn = true, dismissed = true, on = 3, total = 10))
        assertFalse(SetupChecklist.showCard(signedIn = true, dismissed = false, on = 10, total = 10))
    }
}
