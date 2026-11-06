package com.mydomain.android.core

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class SettingsBackupTest {
    /** The "md" preferences as getAll() gives them: settings, plus secrets
     *  and state that must never leave. */
    private val prefs: Map<String, Any?> = mapOf(
        "theme" to "aurora",
        "clip_sync" to true,
        "share_codes" to false,
        "computer_notifs_off" to setOf("node-b", "node-a"),
        "qs_computer" to "node-mac",
        "page_layout" to """{"kinds":{}}""",
        "device_labels" to """{"node-mac":{"nickname":"Work"}}""",
        "server" to "https://example.com",
        "user" to "alice",
        "automation_apps_key" to "SECRET-APPS-KEY",
        "automation_apps_history" to "[\"SECRET-HISTORY\"]",
        "session_token" to "SECRET-SESSION",
        "fcm_token" to "SECRET-FCM",
        "setup_card_dismissed" to true,
        "computer_notifs_capable" to setOf("node-mac"),
    )

    private val automations = """{"automations":[{"id":"a1","trigger":{"kind":"connects","node":"node-mac"},"node":"","action":"notify","args":"{}","label":"Hi","enabled":true}]}"""

    private fun backup(p: Map<String, Any?> = prefs) = SettingsBackup.build(
        prefs = p,
        automations = automations,
        sharing = mapOf("labels_share" to true, "battery_alerts" to mapOf("enabled" to true, "low" to 20L, "charged" to 80L)),
        photoBackup = mapOf("enabled" to true, "target" to "node-mac", "charging_only" to false),
        syncPairs = listOf(mapOf("peer" to "node-mac", "name" to "Docs", "path" to "/sdcard/my-domain/Sync/Docs", "mode" to "both")),
        devices = listOf(SettingsBackup.Device("node-mac", "MacBook")),
        createdAt = 1_700_000_000L,
    )

    @Test
    fun theAllowListHasNoSecrets() {
        for (key in SettingsBackup.PREFS.keys) {
            assertFalse(key, SettingsBackup.looksSecret(key))
            assertTrue(key, SettingsBackup.allowed(key))
        }
        for (key in listOf(
            "automation_apps_key", "server", "user", "session_token", "fcm_token", "identity_key",
            "device_key", "password", "automation_apps_history", "computer_notifs_capable",
        )) {
            assertFalse(key, SettingsBackup.allowed(key))
        }
    }

    @Test
    fun secretsNeverGoInABackup() {
        val text = SettingsBackup.encode(backup())
        assertFalse(text, text.contains("SECRET"))
        for (k in listOf("automation_apps_key", "session_token", "fcm_token", "\"server\"", "\"user\"", "alice", "example.com", "setup_card_dismissed")) {
            assertFalse(k, text.contains(k))
        }
        assertTrue(text.contains("\"clip_sync\":true"))
        assertTrue(text.contains("\"computer_notifs_off\":[\"node-a\",\"node-b\"]"))
    }

    @Test
    fun exportThenImportGivesTheSameSettings() {
        val b = backup()
        val back = SettingsBackup.parse(SettingsBackup.encode(b))
        assertEquals(b, back)
        // Restoring every section writes exactly the allowed keys: a phone
        // that had the backup's settings would export the same backup.
        val written = SettingsBackup.prefsToWrite(back, SettingsBackup.SECTIONS.toSet())
        assertEquals(SettingsBackup.PREFS.keys, written.keys)
        val after = HashMap<String, Any?>(mapOf("server" to "x", "clip_history" to false))
        for ((k, v) in written) {
            if (v == null) after.remove(k) else after[k] = if (v is List<*>) v.toSet() else v
        }
        assertEquals("x", after["server"])
        assertFalse(after.containsKey("clip_history"))
        assertEquals(SettingsBackup.encode(b), SettingsBackup.encode(backup(after)))
        // Nothing would change now.
        val p = SettingsBackup.preview(backup(after), back, setOf("node-mac"))
        assertTrue(p.joinToString { "${it.id}:${it.keys}" }, p.none { it.changed })
    }

    @Test
    fun onlyChosenSectionsAreWritten() {
        val b = backup()
        val w = SettingsBackup.prefsToWrite(b, setOf("appearance"))
        assertEquals(mapOf("theme" to "aurora"), w)
    }

    private fun edited(edit: (MutableMap<String, Any?>) -> Unit): String {
        @Suppress("UNCHECKED_CAST")
        val root = Json.parse(SettingsBackup.encode(backup())) as MutableMap<String, Any?>
        edit(root)
        return Json.write(root)
    }

    @Suppress("UNCHECKED_CAST")
    private fun section(root: MutableMap<String, Any?>, id: String) =
        (root["sections"] as MutableMap<String, Any?>)[id] as MutableMap<String, Any?>

    private fun refused(text: String): SettingsBackup.Problem? =
        (runCatching { SettingsBackup.parse(text) }.exceptionOrNull() as? SettingsBackup.Refused)?.problem

    @Test
    @Suppress("UNCHECKED_CAST")
    fun badFilesAreRefusedWithAReason() {
        assertEquals(SettingsBackup.Problem.NOT_JSON, refused("{ not json"))
        assertEquals(SettingsBackup.Problem.NOT_BACKUP, refused("""{"app":"other"}"""))
        assertEquals(SettingsBackup.Problem.NEWER, refused(edited { it["version"] = 2L }))
        assertEquals(SettingsBackup.Problem.UNKNOWN_VERSION, refused(edited { it["version"] = 0L }))
        assertEquals(SettingsBackup.Problem.FROM_COMPUTER, refused(edited { it["platform"] = "macos" }))
        assertEquals(SettingsBackup.Problem.DAMAGED, refused(edited { it["extra"] = 1L }))
        assertEquals(SettingsBackup.Problem.DAMAGED, refused(edited { section(it, "preferences")["automation_apps_key"] = "abc" }))
        assertEquals(SettingsBackup.Problem.DAMAGED, refused(edited { section(it, "preferences")["server"] = "https://evil" }))
        assertEquals(SettingsBackup.Problem.DAMAGED, refused(edited { section(it, "preferences")["theme"] = "aurora" }))
        assertEquals(SettingsBackup.Problem.DAMAGED, refused(edited { section(it, "preferences")["clip_sync"] = "yes" }))
        assertEquals(SettingsBackup.Problem.DAMAGED, refused(edited { section(it, "automations")["automations"] = "nope" }))
        assertEquals(SettingsBackup.Problem.DAMAGED, refused(edited { section(it, "sharing")["battery_alerts"] = mapOf("enabled" to true, "low" to 0L, "charged" to 0L) }))
        assertEquals(SettingsBackup.Problem.DAMAGED, refused(edited { (section(it, "sync_folders")["pairs"] as MutableList<Any?>).add(mapOf("peer" to "x", "name" to "n", "path" to "/p", "mode" to "mirror")) }))
        assertEquals(SettingsBackup.Problem.DAMAGED, refused(edited { (it["sections"] as MutableMap<String, Any?>)["notes"] = mapOf<String, Any?>() }))
        assertEquals(SettingsBackup.Problem.TOO_BIG, refused(" ".repeat((SettingsBackup.MAX_BYTES + 1).toInt())))
    }

    @Test
    fun sectionsNamingUnknownDevicesStartUnticked() {
        val b = backup()
        val now = backup(emptyMap())
        val p = SettingsBackup.preview(now, b, setOf("node-phone")).associateBy { it.id }
        for (id in listOf("preferences", "automations", "device_labels", "photo_backup", "sync_folders")) {
            assertEquals(id, listOf("MacBook"), p.getValue(id).unknownDevices)
            assertFalse(id, p.getValue(id).on)
        }
        assertTrue(p.getValue("appearance").on)
        assertEquals(listOf("theme"), p.getValue("appearance").keys)
        val known = SettingsBackup.preview(now, b, setOf("node-mac"))
        assertTrue(known.all { it.on })
    }
}
