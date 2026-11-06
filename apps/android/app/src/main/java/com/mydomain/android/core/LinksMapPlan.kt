package com.mydomain.android.core

import kotlin.math.PI
import kotlin.math.cos
import kotlin.math.max
import kotlin.math.roundToInt
import kotlin.math.sin
import kotlin.math.sqrt

/** How this phone is linked to one of your devices (the core's `MapLink`). */
data class MapLinkInfo(
    /** "lan", "direct", "punched", "webrtc", "relay" or "ble". */
    val how: String,
    val transport: String,
    val remote: String? = null,
    /** "local", "ipv6", "mapped", "public". */
    val addrKind: String? = null,
    val rttMs: Int? = null,
    val lossPct: Double? = null,
    val sinceS: Long? = null,
    val heardAgoS: Long = 0,
    /** A media stream runs over it on the local network now (the
     *  "Nearby · low latency" badge). */
    val nearbyMedia: Boolean = false,
    /** That stream's end-to-end latency, when measured. */
    val mediaLatencyMs: Int? = null,
)

/** What stopped one path the last time it was tried. */
data class PathTryInfo(val path: String, val reason: String, val agoS: Long)

/** One of your devices on the map (the core's `MapDevice`). */
data class MapDeviceInfo(
    val nodeId: String,
    val name: String,
    val link: MapLinkInfo?,
    val state: String = link?.how ?: "not_connected",
    val reason: String? = null,
    val lastSeenAgoS: Long? = null,
    val sharesLinks: Boolean = false,
    val tries: List<PathTryInfo> = emptyList(),
    /** "key_changed": its security key changed, refused until you trust
     *  the new one; "bad_key": what it sends can't be read. */
    val problem: String? = null,
)

/** A link between two of your other devices, as one of them said. */
data class OtherLinkInfo(
    val a: String, val b: String, val how: String, val rttMs: Int?,
    val reportedBy: String, val reportedAgoS: Long,
)

/** Everything the map shows (the core's `LinksMap`). */
data class LinksMapInfo(
    val nodeId: String,
    val name: String,
    val devices: List<MapDeviceInfo>,
    val others: List<OtherLinkInfo>,
)

/**
 * The map's picture and words, kept free of Android so it is unit-tested:
 * where each device sits, how each line is drawn, and which reasons to show
 * (the screen turns the codes into strings).
 *
 * This phone sits in the middle; your devices on an ellipse around it, the
 * first at the top (two: left and right), in the core's order (by name), so
 * a device keeps its place as links come and go. Lines between two other
 * devices bow outwards, so they don't run through this phone.
 */
object LinksMapPlan {
    enum class Tone { LAN, DIRECT, RELAY, BLE, NONE }

    /** A point, in the canvas' pixels. */
    data class Pt(val x: Float, val y: Float)

    /** One reason worded by codes: [code] picks the words and the hint
     *  ("lan_blocked"; "unknown" when this app doesn't know it). */
    data class Why(val path: String, val reason: String, val code: String, val agoS: Long?)

    private val REASONS = mapOf(
        "lan" to setOf("no_address", "old_app", "not_seen", "other_network", "blocked"),
        "internet" to setOf("push_failed", "no_answer", "symmetric_nat", "punch_timeout", "no_port_mapping", "timed_out"),
        "relay" to setOf("signed_out", "unreachable", "auth_failed", "relay_full", "peer_offline"),
        "ble" to setOf("off", "not_paired", "out_of_range", "unsupported"),
        "device" to setOf("key_changed", "bad_key"),
    )

    fun tone(how: String?): Tone = when (how) {
        "lan" -> Tone.LAN
        "direct", "punched", "webrtc" -> Tone.DIRECT
        "relay" -> Tone.RELAY
        "ble" -> Tone.BLE
        else -> Tone.NONE
    }

    /** Dash and gap lengths (dp), or null for a solid line: not connected
     *  is dashed, Bluetooth dotted. */
    fun dash(how: String?): FloatArray? = when (tone(how)) {
        Tone.NONE -> floatArrayOf(6f, 6f)
        Tone.BLE -> floatArrayOf(2f, 5f)
        else -> null
    }

    /** A round trip in words: "<1 ms", "4 ms", or null. */
    fun latency(rttMs: Int?): String? = when {
        rttMs == null -> null
        rttMs <= 0 -> "<1 ms"
        else -> "$rttMs ms"
    }

    private fun r1(x: Float) = (x * 10f).roundToInt() / 10f

    /** Where [n] devices sit around the middle of a [w] × [h] canvas,
     *  leaving [mx] / [my] free at the sides for names. */
    fun ring(n: Int, w: Float, h: Float, mx: Float = w * 0.17f, my: Float = h * 0.13f): List<Pt> {
        val cx = w / 2f
        val cy = h / 2f
        val rx = max(cx - mx, 40f)
        val ry = max(cy - my, 40f)
        val start = if (n == 2) PI else -PI / 2
        return (0 until n).map { i ->
            val a = start + 2 * PI * i / n
            Pt(r1(cx + rx * cos(a).toFloat()), r1(cy + ry * sin(a).toFloat()))
        }
    }

    /** The control point of a line from [a] to [b] bowed away from [c],
     *  and where its label sits (the curve's middle). */
    fun bow(a: Pt, b: Pt, c: Pt): Pair<Pt, Pt> {
        val mid = Pt((a.x + b.x) / 2f, (a.y + b.y) / 2f)
        var ox = mid.x - c.x
        var oy = mid.y - c.y
        var len = sqrt(ox * ox + oy * oy)
        if (len < 1f) {
            ox = -(b.y - a.y)
            oy = b.x - a.x
            len = max(sqrt(ox * ox + oy * oy), 1f)
        }
        val span = sqrt((b.x - a.x) * (b.x - a.x) + (b.y - a.y) * (b.y - a.y))
        val k = max(span * 0.3f, 30f)
        val ctrl = Pt(mid.x + ox / len * k, mid.y + oy / len * k)
        val label = Pt((a.x + 2 * ctrl.x + b.x) / 4f, (a.y + 2 * ctrl.y + b.y) / 4f)
        return ctrl to label
    }

    /** The code for [reason] on [path]: "lan_blocked", or "unknown". */
    fun code(path: String, reason: String): String =
        if (REASONS[path]?.contains(reason) == true) "${path}_$reason" else "unknown"

    /** The reasons shown for a device: none while it's linked directly;
     *  else what stopped each path, then the device's own problem. */
    fun whys(d: MapDeviceInfo): List<Why> {
        val how = d.link?.how
        if (how != null && how != "relay" && how != "ble") return emptyList()
        val out = d.tries.map { Why(it.path, it.reason, code(it.path, it.reason), it.agoS) }.toMutableList()
        d.problem?.let { out += Why("device", it, code("device", it), null) }
        return out
    }

    /** Links between other devices that the map can draw: both ends shown. */
    fun drawable(map: LinksMapInfo): List<OtherLinkInfo> {
        val shown = map.devices.map { it.nodeId }.toSet()
        return map.others.filter { it.a in shown && it.b in shown }
    }
}
