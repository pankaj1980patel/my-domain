package com.mydomain.android.core

import android.content.Context
import android.os.Handler
import android.os.Looper
import androidx.compose.runtime.mutableStateMapOf
import org.json.JSONArray
import org.json.JSONObject
import java.io.File
import java.util.concurrent.Executors

/** One reading of a device's battery: unix seconds, percent, charging. */
data class BatterySample(val at: Long, val level: Int, val charging: Boolean)

/** How long until it's empty (or [full]), in minutes. */
data class BatteryEstimate(val full: Boolean, val mins: Double)

/**
 * The last two days of each device's battery, for the graph in its "This
 * device" tile (computers with a battery report one). Like the desktop's
 * `battery_log.rs`: at most one sample per device per 5 minutes, plus one at
 * every change between charging and not; none older than 48 h; kept in
 * `battery_history.json` in the app's files dir, written at most once a
 * minute, so the graph survives a restart.
 */
object BatteryLog {
    private const val MIN_GAP_S = 5 * 60L
    private const val KEEP_S = 48 * 60 * 60L
    /** A device can't grow the file without bound (e.g. a flapping charger). */
    private const val MAX_PER_DEVICE = 1000
    private const val SAVE_DELAY_MS = 60_000L
    private const val FILE = "battery_history.json"

    /** node id -> its samples, oldest first (changed on the main thread). */
    val samples = mutableStateMapOf<String, List<BatterySample>>()

    private val main = Handler(Looper.getMainLooper())
    private val io = Executors.newSingleThreadExecutor()
    private var file: File? = null
    private var savePending = false

    /** Read the saved samples, once (from [Core.init]). */
    @Synchronized
    fun load(context: Context) {
        if (file != null) return
        val f = File(context.filesDir, FILE)
        file = f
        val now = nowS()
        runCatching {
            if (!f.exists()) return
            val o = JSONObject(f.readText())
            for (node in o.keys()) {
                val arr = o.optJSONArray(node) ?: continue
                val list = prune(
                    (0 until arr.length()).mapNotNull { i ->
                        arr.optJSONArray(i)?.let { BatterySample(it.optLong(0), it.optInt(1), it.optInt(2) == 1) }
                    },
                    now,
                )
                if (list.isNotEmpty()) samples[node] = list
            }
        }
    }

    /** A device's status said its battery (main thread). */
    fun record(node: String, level: Int, charging: Boolean) {
        val s = BatterySample(nowS(), level.coerceIn(0, 100), charging)
        val old = samples[node] ?: emptyList()
        val next = prune(add(old, s), s.at)
        if (next == old) return
        samples[node] = next
        scheduleSave()
    }

    /** [list] with [s] added, unless the last sample is under 5 minutes older
     *  and has the same charging state. A sample from before the last one
     *  (the clock went back) replaces the ones after it. */
    fun add(list: List<BatterySample>, s: BatterySample): List<BatterySample> {
        val kept = list.filter { it.at <= s.at }
        val last = kept.lastOrNull()
        if (last != null && last.charging == s.charging && s.at < last.at + MIN_GAP_S) return kept
        return (kept + s).takeLast(MAX_PER_DEVICE)
    }

    /** [list] without the samples more than 48 h older than [now]. */
    fun prune(list: List<BatterySample>, now: Long): List<BatterySample> = list.filter { it.at >= now - KEEP_S }

    /** From the samples of the last hour since it last started or stopped
     *  charging: none when there are too few, they're over 30 minutes old,
     *  span under 20 minutes, or the level barely moves (under 1% an hour).
     *  The same as the desktop's BatteryChart. */
    fun estimate(list: List<BatterySample>, now: Long): BatteryEstimate? {
        val last = list.lastOrNull() ?: return null
        if (now - last.at > 30 * 60) return null
        val recent = list.takeLastWhile { it.at >= last.at - 60 * 60 && it.charging == last.charging }
        if (recent.size < 3 || last.at - recent.first().at < 20 * 60) return null
        // Least squares: percent per hour.
        val mx = recent.sumOf { it.at.toDouble() } / recent.size
        val my = recent.sumOf { it.level.toDouble() } / recent.size
        var num = 0.0
        var den = 0.0
        for (s in recent) {
            num += (s.at - mx) * (s.level - my)
            den += (s.at - mx) * (s.at - mx)
        }
        if (den == 0.0) return null
        val perHour = num / den * 3600
        val mins = if (last.charging) {
            if (perHour < 1 || last.level >= 100) return null
            (100 - last.level) / perHour * 60
        } else {
            if (perHour > -1) return null
            last.level / -perHour * 60
        }
        if (mins < 1 || mins > 48 * 60) return null
        return BatteryEstimate(last.charging, mins)
    }

    fun nowS(): Long = System.currentTimeMillis() / 1000

    /** Write the file a minute from now (once, however many samples come). */
    private fun scheduleSave() {
        if (savePending) return
        savePending = true
        main.postDelayed({
            savePending = false
            val f = file ?: return@postDelayed
            val json = JSONObject().apply {
                samples.forEach { (node, list) ->
                    put(node, JSONArray().apply { list.forEach { put(JSONArray().put(it.at).put(it.level).put(if (it.charging) 1 else 0)) } })
                }
            }.toString()
            io.execute {
                // Write beside it, then swap it in, so a crash can't leave half a file.
                runCatching {
                    val tmp = File(f.path + ".tmp")
                    tmp.writeText(json)
                    if (!tmp.renameTo(f)) {
                        f.writeText(json)
                        tmp.delete()
                    }
                }
            }
        }, SAVE_DELAY_MS)
    }
}
