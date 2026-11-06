package com.mydomain.android.ui

import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.outlined.Info
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.Path
import androidx.compose.ui.graphics.PathEffect
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.drawText
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.rememberTextMeasurer
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.mydomain.android.R
import com.mydomain.android.core.Core
import com.mydomain.android.core.LinksMap
import com.mydomain.android.core.LinksMapInfo
import com.mydomain.android.core.LinksMapPlan
import com.mydomain.android.core.LinksMapPlan.Pt
import com.mydomain.android.core.LinksMapPlan.Tone
import com.mydomain.android.core.MapDeviceInfo
import com.mydomain.android.core.OtherLinkInfo
import com.mydomain.android.core.timeAgo
import kotlinx.coroutines.delay
import kotlin.math.sqrt

/** The Devices screen's way in: "How your devices are connected". */
@Composable
fun LinksMapEntry(onOpen: () -> Unit) {
    if (Core.peers.isEmpty()) return
    Tile(onClick = onOpen) {
        Text(stringResource(R.string.map_entry), style = MaterialTheme.typography.titleMedium, color = T.text)
        Muted(stringResource(R.string.map_entry_body))
    }
}

private fun ago(secs: Long): String = timeAgo(System.currentTimeMillis() - secs * 1000)

@Composable
private fun toneColor(t: Tone): Color = when (t) {
    Tone.LAN -> T.success
    Tone.DIRECT -> T.accent
    Tone.RELAY -> T.warning
    Tone.BLE -> Color(0xFFA78BFA)
    Tone.NONE -> T.text3
}

private fun howRes(how: String?): Int = when (how) {
    "lan" -> R.string.map_how_lan
    "direct" -> R.string.map_how_direct
    "punched" -> R.string.map_how_punched
    "webrtc" -> R.string.map_how_webrtc
    "relay" -> R.string.map_how_relay
    "ble" -> R.string.map_how_ble
    else -> R.string.map_how_none
}

private fun pathRes(how: String): Int? = when (how) {
    "lan" -> R.string.map_path_lan
    "direct" -> R.string.map_path_direct
    "punched" -> R.string.map_path_punched
    "webrtc" -> R.string.map_path_webrtc
    "relay" -> R.string.map_path_relay
    "ble" -> R.string.map_path_ble
    else -> null
}

private fun viaRes(path: String): Int = when (path) {
    "lan" -> R.string.map_via_lan
    "internet" -> R.string.map_via_internet
    "relay" -> R.string.map_via_relay
    "ble" -> R.string.map_via_ble
    else -> R.string.map_via_device
}

private fun addrRes(kind: String?): Int? = when (kind) {
    "local" -> R.string.map_addr_local
    "ipv6" -> R.string.map_addr_ipv6
    "mapped" -> R.string.map_addr_mapped
    "public" -> R.string.map_addr_public
    else -> null
}

private fun encRes(how: String): Int = when (how) {
    "relay" -> R.string.map_enc_relay
    "ble" -> R.string.map_enc_ble
    else -> R.string.map_enc_direct
}

/** A reason's code ([LinksMapPlan.code]) -> (what went wrong, what you can do). */
private val WHY: Map<String, Pair<Int, Int>> = mapOf(
    "lan_no_address" to (R.string.map_why_lan_no_address to R.string.map_fix_lan_no_address),
    "lan_old_app" to (R.string.map_why_lan_old_app to R.string.map_fix_lan_old_app),
    "lan_not_seen" to (R.string.map_why_lan_not_seen to R.string.map_fix_lan_not_seen),
    "lan_other_network" to (R.string.map_why_lan_other_network to R.string.map_fix_lan_other_network),
    "lan_blocked" to (R.string.map_why_lan_blocked to R.string.map_fix_lan_blocked),
    "internet_push_failed" to (R.string.map_why_internet_push_failed to R.string.map_fix_internet_push_failed),
    "internet_no_answer" to (R.string.map_why_internet_no_answer to R.string.map_fix_internet_no_answer),
    "internet_symmetric_nat" to (R.string.map_why_internet_symmetric_nat to R.string.map_fix_internet_symmetric_nat),
    "internet_punch_timeout" to (R.string.map_why_internet_punch_timeout to R.string.map_fix_internet_punch_timeout),
    "internet_no_port_mapping" to (R.string.map_why_internet_no_port_mapping to R.string.map_fix_internet_no_port_mapping),
    "internet_timed_out" to (R.string.map_why_internet_timed_out to R.string.map_fix_internet_timed_out),
    "relay_signed_out" to (R.string.map_why_relay_signed_out to R.string.map_fix_relay_signed_out),
    "relay_unreachable" to (R.string.map_why_relay_unreachable to R.string.map_fix_relay_unreachable),
    "relay_auth_failed" to (R.string.map_why_relay_auth_failed to R.string.map_fix_relay_auth_failed),
    "relay_relay_full" to (R.string.map_why_relay_relay_full to R.string.map_fix_relay_relay_full),
    "relay_peer_offline" to (R.string.map_why_relay_peer_offline to R.string.map_fix_relay_peer_offline),
    "ble_off" to (R.string.map_why_ble_off to R.string.map_fix_ble_off),
    "ble_not_paired" to (R.string.map_why_ble_not_paired to R.string.map_fix_ble_not_paired),
    "ble_out_of_range" to (R.string.map_why_ble_out_of_range to R.string.map_fix_ble_out_of_range),
    "ble_unsupported" to (R.string.map_why_ble_unsupported to R.string.map_fix_ble_unsupported),
    "device_key_changed" to (R.string.map_why_device_key_changed to R.string.map_fix_device_key_changed),
    "device_bad_key" to (R.string.map_why_device_bad_key to R.string.map_fix_device_bad_key),
)

private fun whyRes(code: String): Pair<Int, Int> = WHY[code] ?: (R.string.map_why_unknown to R.string.map_fix_unknown)

/** What was tapped: a device (its line from this phone) or a line between two others. */
private sealed interface Picked
private data class PickedDevice(val node: String) : Picked
private data class PickedOther(val a: String, val b: String) : Picked

/**
 * "How your devices are connected": this phone in the middle, your devices
 * around it, a line per link saying how it's carried (dashed grey: not
 * connected, with when it was last seen), and the links your other devices
 * reported between themselves. Tap a device or a line for the details
 * (and, for one that isn't connected, why, per path, with what you can
 * do); "Connection details" opens that device. The same is listed below
 * the picture, which a screen reader reads as one image.
 */
@Composable
fun LinksMapScreen(onBack: () -> Unit, onHelp: () -> Unit, onDevice: (String) -> Unit) {
    var picked by rememberSaveable { mutableStateOf<String?>(null) }
    LaunchedEffect(LinksMap.changes) {
        delay(150)
        runCatching { LinksMap.load() }
    }
    LaunchedEffect(Unit) {
        while (true) {
            delay(5000)
            runCatching { LinksMap.load() }
        }
    }
    val map = LinksMap.info
    // "d:<node>" or "o:<a>|<b>" (a String survives a rotation).
    val pick: Picked? = picked?.let {
        when {
            it.startsWith("d:") -> PickedDevice(it.removePrefix("d:"))
            it.startsWith("o:") -> it.removePrefix("o:").split("|").takeIf { p -> p.size == 2 }?.let { p -> PickedOther(p[0], p[1]) }
            else -> null
        }
    }

    LazyColumn(
        Modifier.fillMaxSize(),
        contentPadding = PaddingValues(start = 16.dp, end = 16.dp, bottom = 16.dp),
        verticalArrangement = Arrangement.spacedBy(10.dp),
    ) {
        item {
            Row(Modifier.padding(top = 4.dp), verticalAlignment = Alignment.CenterVertically) {
                IconButton(onClick = onBack) {
                    Icon(Icons.AutoMirrored.Filled.ArrowBack, stringResource(R.string.back), tint = T.text)
                }
                Text(
                    stringResource(R.string.map_title), Modifier.weight(1f).semantics { heading() },
                    style = MaterialTheme.typography.headlineSmall, color = T.text,
                )
                IconButton(onClick = onHelp) {
                    Icon(Icons.Outlined.Info, stringResource(R.string.help_open), tint = T.text2)
                }
            }
            Muted(stringResource(R.string.map_subtitle))
        }
        if (map == null) return@LazyColumn
        if (map.devices.isEmpty()) item { Muted(stringResource(R.string.map_empty)) }
        item {
            Tile {
                MapPicture(map, pick, onPick = { picked = it })
                Legend()
            }
        }
        if (pick != null) {
            item { Details(map, pick, onClose = { picked = null }, onDevice = onDevice) }
        }
        item {
            Tile(title = stringResource(R.string.map_list_title)) {
                for (d in map.devices) DeviceRow(d, onDetails = { picked = "d:" + d.nodeId })
                Text(stringResource(R.string.map_others), style = MaterialTheme.typography.titleSmall, color = T.text)
                val others = LinksMapPlan.drawable(map)
                if (others.isEmpty()) Muted(stringResource(R.string.map_others_none))
                for (o in others) OtherRow(map, o, onDetails = { picked = "o:${o.a}|${o.b}" })
                for (d in map.devices.filter { it.link != null && !it.sharesLinks }) {
                    Muted(stringResource(R.string.map_old_app, Core.peerName(d.nodeId)))
                }
            }
        }
    }
}

@Composable
private fun linkLabel(how: String?, rttMs: Int?): String {
    val words = stringResource(howRes(how))
    return LinksMapPlan.latency(rttMs)?.let { "$words · $it" } ?: words
}

@Composable
private fun MapPicture(map: LinksMapInfo, pick: Picked?, onPick: (String) -> Unit) {
    val measurer = rememberTextMeasurer()
    val density = LocalDensity.current
    val me = stringResource(R.string.map_you)
    val names = map.devices.associate { it.nodeId to Core.peerName(it.nodeId) }
    val labels = map.devices.associate { it.nodeId to linkLabel(it.link?.how, it.link?.rttMs) }
    val others = LinksMapPlan.drawable(map)
    val otherLabels = others.associate { "${it.a}|${it.b}" to linkLabel(it.how, it.rttMs) }
    val image = stringResource(R.string.map_image)
    val labelBg = T.surface2
    val nodeBg = T.surface2
    val text = T.text
    val text2 = T.text2
    val accent = T.accent
    val off = T.text3
    val tones = Tone.entries.associateWith { toneColor(it) }
    BoxWithConstraints(Modifier.fillMaxWidth()) {
        val w = with(density) { maxWidth.toPx() }
        val h = w * 0.75f
        val c = Pt(w / 2, h / 2)
        val spots = LinksMapPlan.ring(map.devices.size, w, h)
        val at = map.devices.mapIndexed { i, d -> d.nodeId to spots[i] }.toMap()
        val r = with(density) { 20.dp.toPx() }
        Canvas(
            Modifier
                .fillMaxWidth()
                .height(with(density) { h.toDp() })
                .semantics { contentDescription = image }
                .pointerInput(map) {
                    detectTapGestures { p ->
                        // A device first, then the nearest line.
                        val hit = at.entries.firstOrNull { (_, s) -> dist(p, s) < r * 1.6f }
                        if (hit != null) return@detectTapGestures onPick("d:" + hit.key)
                        var best: String? = null
                        var bestD = with(density) { 24.dp.toPx() }
                        for ((node, s) in at) {
                            val d = toSegment(p, c, s)
                            if (d < bestD) { bestD = d; best = "d:$node" }
                        }
                        for (o in others) {
                            val a = at[o.a] ?: continue
                            val b = at[o.b] ?: continue
                            val (ctrl, _) = LinksMapPlan.bow(a, b, c)
                            val pts = curve(a, ctrl, b)
                            for (i in 0 until pts.size - 1) {
                                val d = toSegment(p, pts[i], pts[i + 1])
                                if (d < bestD) { bestD = d; best = "o:${o.a}|${o.b}" }
                            }
                        }
                        best?.let(onPick)
                    }
                },
        ) {
            val stroke = 2.5.dp.toPx()
            fun effect(how: String?) = LinksMapPlan.dash(how)?.let { d ->
                PathEffect.dashPathEffect(floatArrayOf(d[0].dp.toPx(), d[1].dp.toPx()))
            }
            val style = TextStyle(fontSize = 11.sp, color = text)
            fun label(s: String, x: Float, y: Float, color: Color) {
                val m = measurer.measure(s, style)
                val pad = 6.dp.toPx()
                drawRoundRect(
                    labelBg,
                    topLeft = Offset(x - m.size.width / 2f - pad, y - m.size.height / 2f - 2),
                    size = androidx.compose.ui.geometry.Size(m.size.width + 2 * pad, m.size.height + 4f),
                    cornerRadius = androidx.compose.ui.geometry.CornerRadius(m.size.height.toFloat()),
                )
                drawRoundRect(
                    color,
                    topLeft = Offset(x - m.size.width / 2f - pad, y - m.size.height / 2f - 2),
                    size = androidx.compose.ui.geometry.Size(m.size.width + 2 * pad, m.size.height + 4f),
                    cornerRadius = androidx.compose.ui.geometry.CornerRadius(m.size.height.toFloat()),
                    style = Stroke(1.dp.toPx()),
                )
                drawText(m, topLeft = Offset(x - m.size.width / 2f, y - m.size.height / 2f))
            }
            // This phone's lines.
            for (d in map.devices) {
                val s = at[d.nodeId] ?: continue
                val col = tones.getValue(LinksMapPlan.tone(d.link?.how))
                val thick = if (pick == PickedDevice(d.nodeId)) stroke * 1.6f else stroke
                drawLine(col, Offset(c.x, c.y), Offset(s.x, s.y), thick, StrokeCap.Round, effect(d.link?.how))
            }
            // The lines between your other devices.
            for (o in others) {
                val a = at[o.a] ?: continue
                val b = at[o.b] ?: continue
                val (ctrl, _) = LinksMapPlan.bow(a, b, c)
                val pts = curve(a, ctrl, b)
                val path = Path().apply {
                    moveTo(pts[0].x, pts[0].y)
                    for (p in pts.drop(1)) lineTo(p.x, p.y)
                }
                val thick = if (pick == PickedOther(o.a, o.b)) stroke * 1.6f else stroke
                drawPath(path, tones.getValue(LinksMapPlan.tone(o.how)), style = Stroke(thick, cap = StrokeCap.Round, pathEffect = effect(o.how)))
            }
            // Labels over the lines.
            for (d in map.devices) {
                val s = at[d.nodeId] ?: continue
                label(labels[d.nodeId].orEmpty(), c.x + (s.x - c.x) * 0.56f, c.y + (s.y - c.y) * 0.56f, tones.getValue(LinksMapPlan.tone(d.link?.how)))
            }
            for (o in others) {
                val a = at[o.a] ?: continue
                val b = at[o.b] ?: continue
                val (_, l) = LinksMapPlan.bow(a, b, c)
                label(otherLabels["${o.a}|${o.b}"].orEmpty(), l.x, l.y, tones.getValue(LinksMapPlan.tone(o.how)))
            }
            // The devices, then this phone.
            val nameStyle = TextStyle(fontSize = 12.sp, color = text2)
            val initialStyle = TextStyle(fontSize = 15.sp, color = text, fontWeight = FontWeight.Bold)
            fun node(x: Float, y: Float, rad: Float, name: String, ring: Color, dashed: Boolean, bold: Boolean) {
                drawCircle(nodeBg, rad, Offset(x, y))
                drawCircle(
                    ring, rad, Offset(x, y),
                    style = Stroke(if (bold) 3.dp.toPx() else 2.dp.toPx(), pathEffect = if (dashed) PathEffect.dashPathEffect(floatArrayOf(4.dp.toPx(), 4.dp.toPx())) else null),
                )
                val i = measurer.measure(name.trim().take(1).uppercase().ifEmpty { "?" }, initialStyle)
                drawText(i, topLeft = Offset(x - i.size.width / 2f, y - i.size.height / 2f))
                val n = measurer.measure(name, nameStyle, maxLines = 1, overflow = TextOverflow.Ellipsis, constraints = androidx.compose.ui.unit.Constraints(maxWidth = (rad * 6).toInt()))
                drawText(n, topLeft = Offset(x - n.size.width / 2f, y + rad + 4.dp.toPx()))
            }
            for (d in map.devices) {
                val s = at[d.nodeId] ?: continue
                node(s.x, s.y, r, names[d.nodeId].orEmpty(), if (d.link != null) accent else off, d.link == null, pick == PickedDevice(d.nodeId))
            }
            node(c.x, c.y, r * 1.3f, me, accent, dashed = false, bold = true)
        }
    }
}

private fun dist(p: Offset, s: Pt): Float = sqrt((p.x - s.x) * (p.x - s.x) + (p.y - s.y) * (p.y - s.y))

/** Distance from [p] to the segment [a]–[b]. */
private fun toSegment(p: Offset, a: Pt, b: Pt): Float {
    val dx = b.x - a.x
    val dy = b.y - a.y
    val len2 = dx * dx + dy * dy
    val t = if (len2 == 0f) 0f else (((p.x - a.x) * dx + (p.y - a.y) * dy) / len2).coerceIn(0f, 1f)
    val x = a.x + t * dx
    val y = a.y + t * dy
    return sqrt((p.x - x) * (p.x - x) + (p.y - y) * (p.y - y))
}

/** A quadratic curve from [a] to [b] by [ctrl], as points. */
private fun curve(a: Pt, ctrl: Pt, b: Pt, steps: Int = 20): List<Pt> = (0..steps).map { i ->
    val t = i / steps.toFloat()
    val u = 1 - t
    Pt(u * u * a.x + 2 * u * t * ctrl.x + t * t * b.x, u * u * a.y + 2 * u * t * ctrl.y + t * t * b.y)
}

@Composable
private fun Legend() {
    Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
        for ((tone, how) in listOf(Tone.LAN to "lan", Tone.DIRECT to "direct", Tone.RELAY to "relay", Tone.BLE to "ble", Tone.NONE to null)) {
            Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(4.dp)) {
                Box(Modifier.size(10.dp).background(toneColor(tone), CircleShape))
                Text(stringResource(howRes(how)), style = MaterialTheme.typography.labelSmall, color = T.text2)
            }
        }
    }
}

@Composable
private fun DeviceRow(d: MapDeviceInfo, onDetails: () -> Unit) {
    val name = Core.peerName(d.nodeId)
    val line = if (d.link != null) {
        linkLabel(d.link.how, d.link.rttMs)
    } else {
        val seen = d.lastSeenAgoS?.let { stringResource(R.string.map_last_seen, ago(it)) } ?: stringResource(R.string.map_never_seen)
        stringResource(R.string.map_how_none) + " · " + seen
    }
    val whys = LinksMapPlan.whys(d).map { w ->
        stringResource(viaRes(w.path)) + ": " + stringResource(whyRes(w.code).first, name)
    }
    Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
        Box(Modifier.size(10.dp).background(toneColor(LinksMapPlan.tone(d.link?.how)), CircleShape))
        Column(Modifier.weight(1f)) {
            Text("$name: $line", style = MaterialTheme.typography.bodyMedium, color = T.text)
            if (whys.isNotEmpty()) Muted(whys.joinToString(" · "))
        }
        GhostButton(stringResource(R.string.map_details), onDetails, small = true)
    }
}

@Composable
private fun OtherRow(map: LinksMapInfo, o: OtherLinkInfo, onDetails: () -> Unit) {
    Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
        Box(Modifier.size(10.dp).background(toneColor(LinksMapPlan.tone(o.how)), CircleShape))
        Text(
            stringResource(R.string.map_between, nameOf(map, o.a), nameOf(map, o.b), linkLabel(o.how, o.rttMs)),
            Modifier.weight(1f), style = MaterialTheme.typography.bodyMedium, color = T.text,
        )
        GhostButton(stringResource(R.string.map_details), onDetails, small = true)
    }
}

@Composable
private fun nameOf(map: LinksMapInfo, node: String): String =
    if (node == map.nodeId) stringResource(R.string.map_you) else Core.peerName(node)

@Composable
private fun Kv(key: String, value: String) {
    Column {
        Text(key, style = MaterialTheme.typography.labelMedium, color = T.text3)
        Text(value, style = MaterialTheme.typography.bodyMedium, color = T.text)
    }
}

@Composable
private fun Details(map: LinksMapInfo, pick: Picked, onClose: () -> Unit, onDevice: (String) -> Unit) {
    val me = stringResource(R.string.map_you)
    when (pick) {
        is PickedDevice -> {
            val d = map.devices.firstOrNull { it.nodeId == pick.node } ?: return
            val name = Core.peerName(d.nodeId)
            val l = d.link
            val how = l?.how
            Tile(title = name) {
                Text(stringResource(howRes(how)), color = toneColor(LinksMapPlan.tone(how)), fontWeight = FontWeight.Bold)
                if (d.problem == "key_changed") KeyChangeBanner(d.nodeId, showDismissed = true)
                if (how != null) {
                    pathRes(how)?.let { Kv(stringResource(R.string.map_d_path), stringResource(it, me, name)) }
                }
                if (l != null) {
                    Kv(stringResource(R.string.map_d_transport), stringResource(transportRes(l.transport)))
                    l.remote?.let { remote ->
                        val kind = addrRes(l.addrKind)?.let { " · " + stringResource(it) }.orEmpty()
                        Kv(stringResource(R.string.map_d_address), remote + kind)
                    }
                    l.sinceS?.let { Kv(stringResource(R.string.map_d_since), ago(it)) }
                    Kv(stringResource(R.string.map_d_heard), ago(l.heardAgoS))
                    LinksMapPlan.latency(l.rttMs)?.let { Kv(stringResource(R.string.conn_rtt), it) }
                    Kv(stringResource(R.string.map_d_encryption), stringResource(encRes(l.how)))
                } else {
                    Kv(
                        stringResource(R.string.map_d_last_seen),
                        d.lastSeenAgoS?.let { ago(it) } ?: stringResource(R.string.map_never_seen),
                    )
                }
                val whys = LinksMapPlan.whys(d)
                if (whys.isNotEmpty()) {
                    Text(
                        stringResource(if (l == null) R.string.map_why_title else R.string.map_why_not_direct),
                        Modifier.semantics { heading() }, style = MaterialTheme.typography.titleSmall, color = T.text,
                    )
                    for (w in whys) {
                        val (what, fix) = whyRes(w.code)
                        Column {
                            val when_ = w.agoS?.let { " · " + ago(it) }.orEmpty()
                            Text(stringResource(viaRes(w.path)) + ": " + stringResource(what, name) + when_, style = MaterialTheme.typography.bodyMedium, color = T.text)
                            Muted(stringResource(fix, name))
                        }
                    }
                }
                Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    SoftButton(stringResource(R.string.map_open), { onDevice(d.nodeId) })
                    GhostButton(stringResource(R.string.map_close), onClose)
                }
            }
        }
        is PickedOther -> {
            val o = map.others.firstOrNull { it.a == pick.a && it.b == pick.b } ?: return
            val a = nameOf(map, o.a)
            val b = nameOf(map, o.b)
            Tile(title = stringResource(R.string.map_pair, a, b)) {
                Text(stringResource(howRes(o.how)), color = toneColor(LinksMapPlan.tone(o.how)), fontWeight = FontWeight.Bold)
                pathRes(o.how)?.let { Kv(stringResource(R.string.map_d_path), stringResource(it, a, b)) }
                LinksMapPlan.latency(o.rttMs)?.let { Kv(stringResource(R.string.conn_rtt), it) }
                Kv(stringResource(R.string.map_d_encryption), stringResource(encRes(o.how)))
                Kv(stringResource(R.string.map_d_reported_by), stringResource(R.string.map_reported_value, nameOf(map, o.reportedBy), ago(o.reportedAgoS)))
                Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    SoftButton(stringResource(R.string.map_open_for, a), { onDevice(o.a) })
                    GhostButton(stringResource(R.string.map_close), onClose)
                }
            }
        }
    }
}

private fun transportRes(t: String): Int = when (t) {
    "quic" -> R.string.conn_transport_quic
    "punch" -> R.string.conn_transport_punch
    "webrtc" -> R.string.conn_transport_webrtc
    "relay" -> R.string.conn_transport_relay
    "ble" -> R.string.conn_transport_ble
    else -> R.string.map_how_none
}
