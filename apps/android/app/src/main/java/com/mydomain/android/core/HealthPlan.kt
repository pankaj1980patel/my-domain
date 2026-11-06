package com.mydomain.android.core

/**
 * The health check and first-run setup on the phone, the pure part (no
 * Android types, unit-tested): which "Set up this phone" items each
 * feature group needs and how they count, the facts sent to the core
 * (`health_json`), and what the report says (the badge, a device's
 * problems). The core rates every line (mdcore `features::health`) and
 * runs the stepper (`features::onboarding`); [com.mydomain.android.core.Health]
 * reads the phone and calls it.
 */
object HealthPlan {
    /** The feature groups setup offers, in order (the core's GROUPS). */
    val GROUPS = listOf("clipboard", "notifications", "files", "media", "find")

    /** The steps, in order (the core's STEPS). */
    val STEPS = listOf("name", "devices", "features", "permissions", "connection")

    /** How a "Set up this phone" item counts in the health check. */
    data class Need(val features: List<String>, val critical: Boolean)

    /** Which groups need [item] (empty: everything does) and whether off
     *  breaks them; null: an extra the health check leaves out. */
    fun need(item: SetupItem): Need? = when (item) {
        // Messages, files and rings say so with a notification.
        SetupItem.NOTIFICATIONS -> Need(emptyList(), critical = false)
        SetupItem.NOTIFICATION_ACCESS -> Need(listOf("notifications"), critical = true)
        SetupItem.ACCESSIBILITY_HELPER -> Need(listOf("media"), critical = false)
        // Killed in the background: nothing arrives until it's opened.
        SetupItem.BATTERY -> Need(emptyList(), critical = true)
        SetupItem.AUTOSTART -> Need(emptyList(), critical = false)
        SetupItem.NEARBY_DEVICES -> Need(listOf("files", "media"), critical = false)
        SetupItem.LOCATION_SHARING -> Need(listOf("find"), critical = false)
        SetupItem.LOCATION_PERMISSION -> Need(listOf("find"), critical = false)
        SetupItem.PHOTOS -> Need(listOf("files"), critical = false)
        SetupItem.MICROPHONE -> Need(listOf("media"), critical = false)
        SetupItem.FULL_SCREEN -> Need(listOf("find"), critical = false)
        SetupItem.WRITE_SETTINGS, SetupItem.SHIZUKU, SetupItem.WIDGET, SetupItem.QUICK_TILES -> null
    }

    /** The id the core and the report use for [item]. */
    fun idOf(item: SetupItem): String = item.name.lowercase()

    /** The item a permission line is about (its fix opens it). */
    fun itemOf(id: String?): SetupItem? = SetupItem.entries.firstOrNull { idOf(it) == id }

    /** One permission as sent to the core. */
    data class Fact(val id: String, val features: List<String>, val state: String, val critical: Boolean)

    /** The checklist's rows as the core's permission facts: on is
     *  "granted", off "denied", can't-be-read "unknown"; not needed on
     *  this phone, and the extras, left out. */
    fun facts(rows: List<SetupRow>): List<Fact> = rows.mapNotNull { row ->
        val need = need(row.item) ?: return@mapNotNull null
        val state = when (row.status) {
            SetupStatus.ON -> "granted"
            SetupStatus.OFF -> "denied"
            SetupStatus.INFO -> "unknown"
            SetupStatus.NOT_NEEDED -> return@mapNotNull null
        }
        Fact(idOf(row.item), need.features, state, need.critical)
    }

    /** The items setup's permission step shows for the groups picked:
     *  those that apply here and are needed by a picked group (or by
     *  everything), in the checklist's order. */
    fun forGroups(rows: List<SetupRow>, groups: List<String>): List<SetupRow> = rows.filter { row ->
        val need = need(row.item) ?: return@filter false
        row.status != SetupStatus.NOT_NEEDED && (need.features.isEmpty() || need.features.any { it in groups })
    }

    private fun quote(s: String): String {
        val b = StringBuilder("\"")
        for (c in s) {
            when {
                c == '"' -> b.append("\\\"")
                c == '\\' -> b.append("\\\\")
                c < ' ' -> b.append(String.format("\\u%04x", c.code))
                else -> b.append(c)
            }
        }
        return b.append('"').toString()
    }

    /** The core's `PlatformFacts` as JSON. Free space in MB (null: not known). */
    fun factsJson(facts: List<Fact>, diskFreeMb: Long?, diskTotalMb: Long?): String {
        val perms = facts.joinToString(",") { f ->
            "{\"id\":${quote(f.id)},\"features\":[${f.features.joinToString(",") { quote(it) }}]," +
                "\"state\":${quote(f.state)},\"critical\":${f.critical}}"
        }
        fun mb(v: Long?) = v?.coerceIn(0, UInt.MAX_VALUE.toLong())?.toString() ?: "null"
        return "{\"permissions\":[$perms],\"disk_free_mb\":${mb(diskFreeMb)},\"disk_total_mb\":${mb(diskTotalMb)}}"
    }

    /** One line of the core's report. */
    data class Check(
        val id: String,
        val level: String,
        val reason: String,
        val nodeId: String? = null,
        val name: String? = null,
        val detail: String? = null,
        val features: List<String> = emptyList(),
        val reconnectMs: Long? = null,
        val fix: String? = null,
    ) {
        val bad get() = level == "bad"
        val needsLook get() = level == "bad" || level == "warn"
    }

    /** Red lines (the Home card and the tab badge). */
    fun badCount(checks: List<Check>): Int = checks.count { it.bad }

    /** The amber and red lines about [node] (its screen's banner shows for red). */
    fun deviceProblems(checks: List<Check>, node: String): List<Check> =
        checks.filter { it.nodeId == node && it.needsLook }

    /** The fix button shows: one is named and the line isn't fine. */
    fun showsFix(c: Check): Boolean = c.fix != null && c.level != "ok"

    /** "Step n of 5": the step's number (past the last: the count). */
    fun stepNumber(step: String): Int = STEPS.indexOf(step).let { if (it < 0) STEPS.size else it + 1 }

    /** "1.2 s" / "350 ms". */
    fun millis(ms: Long): String = if (ms >= 1000) String.format(java.util.Locale.ROOT, "%.1f s", ms / 1000.0) else "$ms ms"

    /** A name to start setup's first step with: the phone's own name when
     *  it's a real one, else its model. */
    fun suggestedName(current: String, model: String): String {
        val c = current.trim()
        val looksGeneric = c.isEmpty() || c.equals("localhost", ignoreCase = true) || c.matches(Regex("(?i)android[-_ ]?[0-9a-f]*"))
        return if (looksGeneric && model.isNotBlank()) model.trim() else c
    }
}
