package com.mydomain.android.core

/**
 * Back up and restore this app's settings (Settings → Back up settings /
 * Restore settings…), the part with no Android in it: what may go in the
 * file (one allow-list, [PREFS]), the file's shape, checking a file, and
 * what a restore would change. Plain Kotlin, so the JVM unit tests run it
 * as is; `SettingsBackupTile` reads and writes the phone's settings.
 * Mirrors the desktop's `settings_backup.rs`; the file is the same format
 * (`{"app": "my-domain", "kind": "settings-backup", "version": 1,
 * "platform": "android", "created_at", "devices", "sections"}`).
 *
 * Never in it: the session, device key or any key, the automation-apps key,
 * FCM tokens, passwords, and every message, notification, clipboard item or
 * note. Only [PREFS] keys of the "md" preferences are read at all.
 */
object SettingsBackup {
    const val APP = "my-domain"
    const val KIND = "settings-backup"
    const val VERSION = 1L
    const val PLATFORM = "android"
    /** The biggest backup file read. */
    const val MAX_BYTES = 5L * 1024 * 1024
    /** The longest value of one setting. */
    private const val MAX_VALUE = 1024 * 1024

    enum class Type { BOOL, STRING, STRING_SET }

    /** A "md" preference that may be backed up: its section and type. */
    data class Pref(val section: String, val type: Type)

    /** The allow-list: these "md" preferences and nothing else. */
    val PREFS: Map<String, Pref> = linkedMapOf(
        "theme" to Pref("appearance", Type.STRING),
        "clip_sync" to Pref("preferences", Type.BOOL),
        "clip_history" to Pref("preferences", Type.BOOL),
        "clip_hide_concealed" to Pref("preferences", Type.BOOL),
        "chat_history" to Pref("preferences", Type.BOOL),
        "share_codes" to Pref("preferences", Type.BOOL),
        "touchpad_secure" to Pref("preferences", Type.BOOL),
        "share_location" to Pref("preferences", Type.BOOL),
        "camera_keep_locked" to Pref("preferences", Type.BOOL),
        "nearby_media" to Pref("preferences", Type.BOOL),
        // The switch only: the key itself is in the Keystore, never here.
        "automation_apps" to Pref("preferences", Type.BOOL),
        "computer_notifs_off" to Pref("preferences", Type.STRING_SET),
        "qs_computer" to Pref("preferences", Type.STRING),
        "page_layout" to Pref("page_layout", Type.STRING),
        "device_labels" to Pref("device_labels", Type.STRING),
    )

    /** Every section, in the order a restore lists them. */
    val SECTIONS = listOf(
        "appearance", "preferences", "sharing", "automations", "device_labels",
        "page_layout", "photo_backup", "sync_folders",
    )

    /** A belt to the allow-list's braces: a key named like a secret never
     *  goes out or comes in, whatever list it's on. */
    fun looksSecret(key: String): Boolean {
        val k = key.lowercase()
        return listOf(
            "token", "secret", "password", "passwd", "credential", "creds", "identity_key",
            "device_key", "private", "session", "fcm", "_key",
        ).any { k.contains(it) } || k == "key" || k == "server" || k == "user"
    }

    /** Whether "md" preference [key] may be backed up (and restored). */
    fun allowed(key: String): Boolean = key in PREFS && !looksSecret(key)

    /** The allowed preferences of [all] (`SharedPreferences.getAll()`), by
     *  section: key -> Boolean, String, or a sorted list for a string set. */
    fun pickPrefs(all: Map<String, *>): Map<String, Map<String, Any>> {
        val out = linkedMapOf<String, LinkedHashMap<String, Any>>()
        for ((key, pref) in PREFS) {
            if (!allowed(key)) continue
            val v = all[key] ?: continue
            val value: Any = when (pref.type) {
                Type.BOOL -> v as? Boolean
                Type.STRING -> v as? String
                Type.STRING_SET -> (v as? Set<*>)?.filterIsInstance<String>()?.sorted()
            } ?: continue
            out.getOrPut(pref.section) { linkedMapOf() }[key] = value
        }
        return out
    }

    // --- the file ---

    data class Device(val id: String, val name: String)

    data class Backup(
        val platform: String,
        val createdAt: Long,
        val devices: List<Device>,
        /** Section id -> its settings (JSON values: Boolean, String, Long,
         *  List, Map). */
        val sections: Map<String, Map<String, Any?>>,
    )

    /** Everything a backup holds, from the phone's settings now. Every
     *  preference section is there (empty: all defaults). */
    fun build(
        prefs: Map<String, *>,
        automations: String?,
        sharing: Map<String, Any?>?,
        photoBackup: Map<String, Any?>?,
        syncPairs: List<Map<String, Any?>>?,
        devices: List<Device>,
        createdAt: Long,
    ): Backup {
        val picked = pickPrefs(prefs)
        val sections = linkedMapOf<String, Map<String, Any?>>()
        for (id in SECTIONS) {
            when (id) {
                "automations" -> if (automations != null) sections[id] = mapOf("automations" to automations)
                "sharing" -> if (sharing != null) sections[id] = sharing
                "photo_backup" -> if (photoBackup != null) sections[id] = photoBackup
                "sync_folders" -> if (syncPairs != null) sections[id] = mapOf("pairs" to syncPairs)
                else -> sections[id] = picked[id] ?: emptyMap()
            }
        }
        return Backup(PLATFORM, createdAt, devices, sections)
    }

    fun encode(b: Backup): String = Json.write(
        linkedMapOf(
            "app" to APP,
            "kind" to KIND,
            "version" to VERSION,
            "platform" to b.platform,
            "created_at" to b.createdAt,
            "devices" to b.devices.map { linkedMapOf("id" to it.id, "name" to it.name) },
            "sections" to b.sections,
        ),
    )

    /** Why a file can't be restored. */
    enum class Problem { TOO_BIG, NOT_JSON, NOT_BACKUP, NEWER, UNKNOWN_VERSION, FROM_COMPUTER, DAMAGED }

    class Refused(val problem: Problem, detail: String = "") : Exception("$problem $detail".trim())

    /** The backup in [text], checked; throws [Refused] saying why not. */
    fun parse(text: String): Backup {
        if (text.length.toLong() > MAX_BYTES) throw Refused(Problem.TOO_BIG)
        val root = runCatching { Json.parse(text) }.getOrNull() as? Map<*, *> ?: throw Refused(Problem.NOT_JSON)
        if (root["app"] != APP || root["kind"] != KIND) throw Refused(Problem.NOT_BACKUP)
        val version = root["version"] as? Long ?: throw Refused(Problem.UNKNOWN_VERSION)
        if (version > VERSION) throw Refused(Problem.NEWER, "$version")
        if (version != VERSION) throw Refused(Problem.UNKNOWN_VERSION)
        val platform = root["platform"] as? String
        if (platform in setOf("macos", "windows", "linux")) throw Refused(Problem.FROM_COMPUTER)
        if (platform != PLATFORM) throw Refused(Problem.NOT_BACKUP)
        val known = setOf("app", "kind", "version", "platform", "created_at", "devices", "sections")
        root.keys.firstOrNull { it !in known }?.let { damaged("unexpected \"$it\"") }
        val createdAt = root["created_at"] as? Long ?: damaged("created_at")
        val devices = (root["devices"] as? List<*> ?: emptyList<Any>()).map { d ->
            val m = d as? Map<*, *> ?: damaged("devices")
            Device(m["id"] as? String ?: damaged("devices"), m["name"] as? String ?: damaged("devices"))
        }
        val rawSections = root["sections"] as? Map<*, *> ?: damaged("sections")
        val sections = linkedMapOf<String, Map<String, Any?>>()
        for ((k, v) in rawSections) {
            val id = k as String
            if (id !in SECTIONS) damaged("unexpected section \"$id\"")
            val m = (v as? Map<*, *> ?: damaged(id)).entries.associate { (kk, vv) -> kk as String to vv }
            check(id, m)
            sections[id] = m
        }
        return Backup(PLATFORM, createdAt, devices, sections)
    }

    private fun damaged(why: String): Nothing = throw Refused(Problem.DAMAGED, why)

    private fun check(id: String, m: Map<String, Any?>) {
        fun only(vararg keys: String) = m.keys.firstOrNull { it !in keys }?.let { damaged("$id: unexpected \"$it\"") }
        fun bool(k: String) = m[k] as? Boolean ?: damaged("$id: $k")
        fun str(k: String) = (m[k] as? String)?.takeIf { it.length <= MAX_VALUE } ?: damaged("$id: $k")
        when (id) {
            "appearance", "preferences", "page_layout", "device_labels" -> for ((k, v) in m) {
                val pref = PREFS[k]
                if (!allowed(k) || pref == null || pref.section != id) damaged("$id: unexpected \"$k\"")
                val ok = when (pref.type) {
                    Type.BOOL -> v is Boolean
                    Type.STRING -> v is String && v.length <= MAX_VALUE
                    Type.STRING_SET -> v is List<*> && v.all { it is String }
                }
                if (!ok) damaged("$id: $k")
                if (k == "theme" && !(v as String).all { it.isLetterOrDigit() || it == '-' || it == '_' }) damaged("theme")
                if (k == "page_layout" || k == "device_labels") {
                    if (runCatching { Json.parse(v as String) }.getOrNull() !is Map<*, *>) damaged("$id: $k")
                }
            }
            "automations" -> {
                only("automations")
                val root = runCatching { Json.parse(str("automations")) }.getOrNull() as? Map<*, *>
                if (root?.get("automations") !is List<*>) damaged("automations")
            }
            "sharing" -> {
                only("labels_share", "battery_alerts")
                bool("labels_share")
                val b = m["battery_alerts"] as? Map<*, *> ?: damaged("battery_alerts")
                val low = b["low"] as? Long
                val charged = b["charged"] as? Long
                if (b["enabled"] !is Boolean || low == null || low !in 1..99 || charged == null || charged !in 0..100) {
                    damaged("battery_alerts")
                }
            }
            "photo_backup" -> {
                only("enabled", "target", "charging_only")
                bool("enabled"); str("target"); bool("charging_only")
            }
            "sync_folders" -> {
                only("pairs")
                val pairs = m["pairs"] as? List<*> ?: damaged("sync_folders")
                for (p in pairs) {
                    val o = p as? Map<*, *> ?: damaged("sync_folders")
                    if (o.keys.any { it !in setOf("peer", "name", "path", "mode") }) damaged("sync_folders")
                    val peer = o["peer"] as? String
                    val path = o["path"] as? String
                    if (peer.isNullOrEmpty() || path.isNullOrEmpty() || o["name"] !is String) damaged("sync_folders")
                    if (o["mode"] !in setOf("both", "send", "receive")) damaged("sync_folders: mode")
                }
            }
        }
    }

    // --- what a restore would change ---

    data class SectionPreview(
        val id: String,
        /** The settings that would change, or the synced folders added. */
        val keys: List<String>,
        val unknownDevices: List<String>,
        /** Ticked to start with. */
        val on: Boolean,
    ) {
        val changed get() = keys.isNotEmpty()
    }

    /** The devices (names) [text] names that aren't [known] here (only the
     *  backup's own devices are looked for). */
    fun unknownDevices(text: String, devices: List<Device>, known: Set<String>): List<String> =
        devices.filter { it.id.isNotEmpty() && text.contains(it.id) && it.id !in known }
            .map { it.name.ifEmpty { it.id } }

    /** The synced folders of [then] not here [now] (same device and folder). */
    fun newPairs(then: Map<String, Any?>?, now: Map<String, Any?>?): List<Map<*, *>> {
        val have = (now?.get("pairs") as? List<*>).orEmpty().mapNotNull { it as? Map<*, *> }
        return (then?.get("pairs") as? List<*>).orEmpty().mapNotNull { it as? Map<*, *> }
            .filter { p -> have.none { it["peer"] == p["peer"] && it["path"] == p["path"] } }
    }

    /** What restoring [b] over [now] (the phone's backup now) would change. */
    fun preview(now: Backup, b: Backup, known: Set<String>): List<SectionPreview> = SECTIONS.mapNotNull { id ->
        val then = b.sections[id] ?: return@mapNotNull null
        val cur = now.sections[id].orEmpty()
        val keys = if (id == "sync_folders") {
            newPairs(then, cur).map { it["name"] as? String ?: "" }
        } else {
            // A preference section sets the allowed keys it has, and clears
            // the ones it hasn't; the others replace what they hold.
            (cur.keys + then.keys).filter { cur[it] != then[it] }
        }
        val unknown = unknownDevices(Json.write(then), b.devices, known)
        SectionPreview(id, keys, unknown, unknown.isEmpty())
    }

    /** The "md" preferences restoring [chosen] of [b] writes: key -> value,
     *  or null to clear it (back to its default). Only allowed keys. */
    fun prefsToWrite(b: Backup, chosen: Set<String>): Map<String, Any?> {
        val out = linkedMapOf<String, Any?>()
        for ((key, pref) in PREFS) {
            if (!allowed(key) || pref.section !in chosen) continue
            val section = b.sections[pref.section] ?: continue
            out[key] = section[key]
        }
        return out
    }
}
