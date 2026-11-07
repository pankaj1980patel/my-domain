package com.mydomain.android.core

import android.content.Context
import android.os.StatFs
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import com.mydomain.android.FileActions
import com.mydomain.android.SetupState
import org.json.JSONArray
import org.json.JSONObject

/**
 * The health check and first-run setup on the phone: reads the checklist
 * and free space, asks the core to rate everything (`health_json`), and
 * keeps the report and where setup stands for the screens (Compose state).
 * The rules are the core's and [HealthPlan]'s.
 */
object Health {
    /** Where setup stands (the core's `Onboarding`). */
    data class Setup(
        val step: String,
        val finished: Boolean,
        val skippedAll: Boolean,
        val skipped: List<String>,
        val features: List<String>?,
    ) {
        /** The groups in use: those picked, else all. */
        val enabled get() = features ?: HealthPlan.GROUPS
    }

    var checks by mutableStateOf<List<HealthPlan.Check>>(emptyList()); private set
    var running by mutableStateOf(false); private set
    /** When the report was made (ms since the epoch; 0: never). */
    var ranAt by mutableStateOf(0L); private set
    var setup by mutableStateOf<Setup?>(null); private set

    /** Signed in or joined just now: the app shell opens setup once. */
    var justSignedIn by mutableStateOf(false)

    val bad get() = HealthPlan.badCount(checks)

    private fun strings(a: JSONArray?): List<String> =
        if (a == null) emptyList() else (0 until a.length()).map { a.optString(it) }

    private fun optStr(o: JSONObject, k: String): String? = if (o.isNull(k)) null else o.optString(k)

    private fun parseSetup(json: String): Setup {
        val o = JSONObject(json)
        return Setup(
            step = o.optString("step", "name"),
            finished = o.optBoolean("finished"),
            skippedAll = o.optBoolean("skipped_all"),
            skipped = strings(o.optJSONArray("skipped")),
            features = if (o.isNull("features")) null else strings(o.optJSONArray("features")),
        )
    }

    private fun parseReport(json: String): List<HealthPlan.Check> {
        val a = JSONObject(json).optJSONArray("checks") ?: return emptyList()
        return (0 until a.length()).map { i ->
            val c = a.getJSONObject(i)
            HealthPlan.Check(
                id = c.optString("id"),
                level = c.optString("level"),
                reason = c.optString("reason"),
                nodeId = optStr(c, "node_id"),
                name = optStr(c, "name"),
                detail = optStr(c, "detail"),
                features = strings(c.optJSONArray("features")),
                reconnectMs = if (c.isNull("reconnect_ms")) null else c.optLong("reconnect_ms"),
                fix = optStr(c, "fix"),
            )
        }
    }

    /** What only the phone can read, as the core's `PlatformFacts`. */
    private fun facts(ctx: Context): String {
        val rows = SetupState.rows(ctx)
        val disk = runCatching {
            val s = StatFs(FileActions.downloadDir(ctx).absolutePath)
            s.availableBytes / (1024 * 1024) to s.totalBytes / (1024 * 1024)
        }.getOrNull()
        return HealthPlan.factsJson(HealthPlan.facts(rows), disk?.first, disk?.second)
    }

    /** Sample mode (debug screenshots): this report instead; empty when it
     *  ends (the real one runs again). */
    fun sample(list: List<HealthPlan.Check>) {
        checks = list
        ranAt = if (list.isEmpty()) 0L else System.currentTimeMillis()
    }

    /** Run the check (`server`: ask the server too, a few seconds). */
    suspend fun run(ctx: Context, server: Boolean = true) {
        if (Core.engine() == null) return
        running = true
        try {
            val json = Core.call { it.healthJson(facts(ctx.applicationContext), server) }
            var next = parseReport(json)
            // A quick run keeps the last server line.
            if (!server) checks.firstOrNull { it.id == "server" }?.let { s ->
                if (next.none { it.id == "server" }) next = listOf(s) + next
            }
            checks = next.sortedByDescending { levelRank(it.level) }
            ranAt = System.currentTimeMillis()
        } catch (_: Exception) {
            // Not signed in or the engine is gone: keep what was shown.
        } finally {
            running = false
        }
    }

    private fun levelRank(level: String) = when (level) {
        "bad" -> 3
        "warn" -> 2
        "info" -> 1
        else -> 0
    }

    suspend fun loadSetup(): Setup? = runCatching {
        parseSetup(Core.call { it.onboardingJson() }).also { setup = it }
    }.getOrNull()

    /** A tap in setup: "next", "back", "skip", "goto" (with [step]),
     *  "skip_all", "finish", "restart". */
    suspend fun act(action: String, step: String? = null): Setup =
        parseSetup(Core.call { it.onboardingAct(action, step) }).also { setup = it }

    suspend fun setFeatures(groups: List<String>): Setup =
        parseSetup(Core.call { it.setOnboardingFeatures(groups) }).also { setup = it }

    /** Name this phone; returns the name as kept. */
    suspend fun setName(name: String): String {
        val kept = Core.call { it.setDeviceName(name) }
        Core.reload()
        return kept
    }

    /** The health check's "Test push". */
    suspend fun pushTest() {
        Core.call { it.fcmSelftest() }
    }
}
