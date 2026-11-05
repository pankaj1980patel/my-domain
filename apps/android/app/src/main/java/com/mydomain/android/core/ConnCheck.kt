package com.mydomain.android.core

import androidx.compose.runtime.mutableStateMapOf
import org.json.JSONObject

/**
 * "Check connection" on a computer's screen: the core's report on the link
 * to it (`diagnose`, JSON) and its speed test, whose progress arrives as
 * `speed_test` events ([onSpeedEvent], from [Core]'s event handling).
 */
object ConnCheck {
    /** A report and when it was made (ms since the epoch). */
    data class Report(val at: Long, val json: JSONObject)

    /** One speed test, as its events said. `result` once it's done. */
    data class SpeedRun(
        val id: String,
        val phase: String,
        val done: Long,
        val total: Long,
        val result: JSONObject? = null,
        val error: String? = null,
    ) {
        val running get() = phase == "ping" || phase == "upload" || phase == "download"
    }

    /** node id -> its last report. */
    val reports = mutableStateMapOf<String, Report>()

    /** node id -> its current or last speed test. */
    val speed = mutableStateMapOf<String, SpeedRun>()

    /** Ask for [node]'s report (a few seconds: it pings and checks the relay). */
    suspend fun diagnose(node: String): JSONObject {
        val json = JSONObject(Core.call { it.diagnose(node) })
        reports[node] = Report(System.currentTimeMillis(), json)
        return json
    }

    /** Bytes each way: less over the relay or Bluetooth. */
    fun bytesFor(transport: String?): Int =
        if (transport == "relay" || transport == "ble") 2 * 1024 * 1024 else 8 * 1024 * 1024

    suspend fun startSpeedTest(node: String, bytes: Int) {
        val id = Core.call { it.speedTest(node, bytes.toUInt()) }
        // Its first events may have come already.
        if (speed[node]?.id != id) speed[node] = SpeedRun(id, "ping", 0, bytes.toLong())
    }

    suspend fun cancelSpeedTest(node: String) {
        val run = speed[node] ?: return
        Core.call { it.cancelSpeedTest(run.id) }
    }

    /** A `speed_test` event (on the main thread). A test still running for
     *  the device wins over any other. */
    fun onSpeedEvent(o: JSONObject) {
        val node = o.optString("node_id")
        val id = o.optString("id")
        val cur = speed[node]
        if (cur != null && cur.id != id && cur.running) return
        speed[node] = SpeedRun(
            id = id,
            phase = o.optString("phase"),
            done = o.optLong("done"),
            total = o.optLong("total"),
            result = o.optJSONObject("result"),
            error = if (o.isNull("error")) null else o.optString("error"),
        )
    }
}
