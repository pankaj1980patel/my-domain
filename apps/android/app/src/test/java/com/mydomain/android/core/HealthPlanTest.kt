package com.mydomain.android.core

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class HealthPlanTest {

    private fun row(item: SetupItem, status: SetupStatus) = SetupRow(item, status)

    @Test
    fun checklistRowsBecomeFacts() {
        val rows = listOf(
            row(SetupItem.NOTIFICATION_ACCESS, SetupStatus.OFF),
            row(SetupItem.BATTERY, SetupStatus.ON),
            row(SetupItem.AUTOSTART, SetupStatus.INFO),
            row(SetupItem.NEARBY_DEVICES, SetupStatus.NOT_NEEDED),
            row(SetupItem.WIDGET, SetupStatus.OFF),
            row(SetupItem.MICROPHONE, SetupStatus.OFF),
        )
        val facts = HealthPlan.facts(rows)
        assertEquals(
            listOf("notification_access" to "denied", "battery" to "granted", "autostart" to "unknown", "microphone" to "denied"),
            facts.map { it.id to it.state },
        )
        // Notification access breaks notifications; the battery breaks everything.
        assertTrue(facts.first { it.id == "notification_access" }.critical)
        assertEquals(listOf("notifications"), facts.first { it.id == "notification_access" }.features)
        assertTrue(facts.first { it.id == "battery" }.features.isEmpty())
        assertFalse(facts.first { it.id == "microphone" }.critical)
    }

    @Test
    fun everyItemHasAnIdThatComesBack() {
        for (item in SetupItem.entries) {
            assertEquals(item, HealthPlan.itemOf(HealthPlan.idOf(item)))
        }
        assertNull(HealthPlan.itemOf("bogus"))
        assertNull(HealthPlan.itemOf(null))
        // The extras aren't health checks.
        assertNull(HealthPlan.need(SetupItem.WIDGET))
        assertNull(HealthPlan.need(SetupItem.SHIZUKU))
    }

    @Test
    fun thePermissionStepShowsWhatThePickedGroupsNeed() {
        val rows = SetupItem.entries.map {
            row(it, if (it == SetupItem.FULL_SCREEN) SetupStatus.NOT_NEEDED else SetupStatus.OFF)
        }
        val shown = HealthPlan.forGroups(rows, listOf("clipboard")).map { it.item }
        // Clipboard alone: only what everything needs.
        assertEquals(listOf(SetupItem.NOTIFICATIONS, SetupItem.BATTERY, SetupItem.AUTOSTART), shown)
        val media = HealthPlan.forGroups(rows, listOf("media")).map { it.item }
        assertTrue(SetupItem.MICROPHONE in media)
        assertTrue(SetupItem.ACCESSIBILITY_HELPER in media)
        assertFalse(SetupItem.PHOTOS in media)
        assertFalse(SetupItem.WIDGET in media)
        // Not needed on this phone: never shown.
        val find = HealthPlan.forGroups(rows, listOf("find")).map { it.item }
        assertTrue(SetupItem.LOCATION_PERMISSION in find)
        assertFalse(SetupItem.FULL_SCREEN in find)
    }

    @Test
    fun factsAsJson() {
        val json = HealthPlan.factsJson(
            listOf(HealthPlan.Fact("battery", emptyList(), "granted", true), HealthPlan.Fact("a\"b", listOf("media", "find"), "denied", false)),
            diskFreeMb = 2048,
            diskTotalMb = null,
        )
        assertEquals(
            "{\"permissions\":[{\"id\":\"battery\",\"features\":[],\"state\":\"granted\",\"critical\":true}," +
                "{\"id\":\"a\\\"b\",\"features\":[\"media\",\"find\"],\"state\":\"denied\",\"critical\":false}]," +
                "\"disk_free_mb\":2048,\"disk_total_mb\":null}",
            json,
        )
        // Too big for the core's u32: capped.
        assertTrue(HealthPlan.factsJson(emptyList(), Long.MAX_VALUE, -5).contains("\"disk_free_mb\":4294967295,\"disk_total_mb\":0"))
    }

    @Test
    fun theReportsBadgeAndADevicesProblems() {
        val checks = listOf(
            HealthPlan.Check("server", "ok", "ok"),
            HealthPlan.Check("link", "warn", "relay", nodeId = "pc", fix = "links_map"),
            HealthPlan.Check("key", "bad", "changed", nodeId = "pc", fix = "trust_key"),
            HealthPlan.Check("version", "info", "older", nodeId = "tab", fix = "update_other"),
            HealthPlan.Check("disk", "bad", "full", fix = "free_space"),
        )
        assertEquals(2, HealthPlan.badCount(checks))
        assertEquals(listOf("link", "key"), HealthPlan.deviceProblems(checks, "pc").map { it.id })
        assertTrue(HealthPlan.deviceProblems(checks, "tab").isEmpty())
        assertFalse(HealthPlan.showsFix(checks[0]))
        assertTrue(HealthPlan.showsFix(checks[3]))
        assertFalse(HealthPlan.showsFix(HealthPlan.Check("push", "ok", "working", fix = "push_test")))
    }

    @Test
    fun stepsNumbersAndNames() {
        assertEquals(1, HealthPlan.stepNumber("name"))
        assertEquals(5, HealthPlan.stepNumber("connection"))
        assertEquals(5, HealthPlan.stepNumber("done"))
        assertEquals("350 ms", HealthPlan.millis(350))
        assertEquals("1.5 s", HealthPlan.millis(1500))
        assertEquals("Pixel 8", HealthPlan.suggestedName("android-3f2a9c", "Pixel 8"))
        assertEquals("Pixel 8", HealthPlan.suggestedName("  ", "Pixel 8"))
        assertEquals("Pankaj’s phone", HealthPlan.suggestedName("Pankaj’s phone", "Pixel 8"))
        assertEquals("localhost", HealthPlan.suggestedName("localhost", ""))
    }
}
