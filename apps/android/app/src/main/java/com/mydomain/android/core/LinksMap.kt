package com.mydomain.android.core

import android.bluetooth.BluetoothManager
import android.content.Context
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import org.json.JSONArray
import org.json.JSONObject

/**
 * "How your devices are connected": the core's map (`links_map`, JSON) read
 * into [LinksMapInfo]. [changes] goes up when a link comes up or goes down
 * here, a device sends its own links (`links_map_changed`) or the device
 * list changes ([Core] calls [changed]); the screen reads the map again
 * then, and every few seconds for the round trips. Before reading, the
 * core is told whether this phone's Bluetooth can link at all, so a device
 * that isn't connected can say "Bluetooth is off".
 */
object LinksMap {
    var info by mutableStateOf<LinksMapInfo?>(null)
        private set
    var changes by mutableIntStateOf(0)
        private set

    fun changed() {
        changes++
    }

    /** Read the map again (off the main thread). */
    suspend fun load() {
        val ble = bluetoothState(Core.app)
        val json = Core.call { e ->
            runCatching { e.setBleState(ble) }
            e.linksMap()
        }
        info = parse(JSONObject(json))
    }

    /** "off" or "unsupported" when this phone's Bluetooth can't link; null when it can. */
    private fun bluetoothState(ctx: Context): String? = runCatching {
        val adapter = (ctx.getSystemService(Context.BLUETOOTH_SERVICE) as? BluetoothManager)?.adapter
        when {
            adapter == null -> "unsupported"
            !adapter.isEnabled -> "off"
            else -> null
        }
    }.getOrNull()

    private fun JSONObject.strOrNull(k: String): String? = if (isNull(k) || !has(k)) null else optString(k)
    private fun JSONObject.longOrNull(k: String): Long? = if (isNull(k) || !has(k)) null else optLong(k)
    private fun JSONObject.intOrNull(k: String): Int? = if (isNull(k) || !has(k)) null else optInt(k)
    private fun JSONObject.doubleOrNull(k: String): Double? = if (isNull(k) || !has(k)) null else optDouble(k)
    private fun JSONArray?.objects(): List<JSONObject> =
        if (this == null) emptyList() else (0 until length()).mapNotNull { optJSONObject(it) }

    fun parse(o: JSONObject): LinksMapInfo = LinksMapInfo(
        nodeId = o.optString("node_id"),
        name = o.optString("name"),
        devices = o.optJSONArray("devices").objects().map { d ->
            val link = d.optJSONObject("link")?.let { l ->
                MapLinkInfo(
                    how = l.optString("how"),
                    transport = l.optString("transport"),
                    remote = l.strOrNull("remote"),
                    addrKind = l.strOrNull("addr_kind"),
                    rttMs = l.intOrNull("rtt_ms"),
                    lossPct = l.doubleOrNull("loss_pct"),
                    sinceS = l.longOrNull("since_s"),
                    heardAgoS = l.optLong("heard_ago_s"),
                    nearbyMedia = l.optBoolean("nearby_media"),
                    mediaLatencyMs = l.intOrNull("media_latency_ms"),
                )
            }
            MapDeviceInfo(
                nodeId = d.optString("node_id"),
                name = d.optString("name"),
                link = link,
                state = d.optString("state"),
                reason = d.strOrNull("reason"),
                lastSeenAgoS = d.longOrNull("last_seen_ago_s"),
                sharesLinks = d.optBoolean("shares_links"),
                tries = d.optJSONArray("tries").objects().map {
                    PathTryInfo(it.optString("path"), it.optString("reason"), it.optLong("ago_s"))
                },
                problem = d.strOrNull("problem"),
                verified = d.optBoolean("verified"),
            )
        },
        others = o.optJSONArray("others").objects().map {
            OtherLinkInfo(
                a = it.optString("a"), b = it.optString("b"), how = it.optString("how"),
                rttMs = it.intOrNull("rtt_ms"), reportedBy = it.optString("reported_by"),
                reportedAgoS = it.optLong("reported_ago_s"),
            )
        },
    )
}
