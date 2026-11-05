package com.mydomain.android.ui

import android.text.format.Formatter
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import com.mydomain.android.R
import com.mydomain.android.core.ConnCheck
import com.mydomain.android.core.Core
import com.mydomain.android.core.friendlyError
import com.mydomain.android.core.timeAgo
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.launch
import org.json.JSONObject

/** A string field, or null when absent or JSON null. */
private fun JSONObject.str(key: String): String? = if (isNull(key)) null else optString(key)
private fun JSONObject.num(key: String): Double? = if (isNull(key)) null else optDouble(key)

private fun ms(x: Double?): String = when {
    x == null -> "–"
    x < 10 -> String.format(java.util.Locale.getDefault(), "%.1f", x)
    else -> Math.round(x).toString()
}

private fun agoText(secs: Long): String = timeAgo(System.currentTimeMillis() - secs * 1000)

private fun verdictRes(v: String): Int? = when (v) {
    "lan" -> R.string.conn_verdict_lan
    "direct" -> R.string.conn_verdict_direct
    "punched" -> R.string.conn_verdict_punched
    "webrtc" -> R.string.conn_verdict_webrtc
    "relay" -> R.string.conn_verdict_relay
    "ble" -> R.string.conn_verdict_ble
    else -> null
}

private fun reasonRes(r: String?): Int? = when (r) {
    "push_failed" -> R.string.conn_reason_push_failed
    "no_answer" -> R.string.conn_reason_no_answer
    "relay_no_answer" -> R.string.conn_reason_relay_no_answer
    "relay_unreachable" -> R.string.conn_reason_relay_unreachable
    "no_path" -> R.string.conn_reason_no_path
    "dropped" -> R.string.conn_reason_dropped
    else -> null
}

private fun stageRes(s: String): Int? = when (s) {
    "quic" -> R.string.conn_stage_quic
    "signaled_quic" -> R.string.conn_stage_signaled_quic
    "relay" -> R.string.conn_stage_relay
    "punching" -> R.string.conn_stage_punching
    "webrtc" -> R.string.conn_stage_webrtc
    "nat_skip" -> R.string.conn_stage_nat_skip
    "connected" -> R.string.conn_stage_connected
    "failed" -> R.string.conn_stage_failed
    "push_failed" -> R.string.conn_stage_push_failed
    "no_answer" -> R.string.conn_stage_no_answer
    "relay_no_answer" -> R.string.conn_stage_relay_no_answer
    "relay_unreachable" -> R.string.conn_stage_relay_unreachable
    "link_up" -> R.string.conn_stage_link_up
    "link_down" -> R.string.conn_stage_link_down
    else -> null
}

private fun transportRes(t: String?): Int? = when (t) {
    "quic" -> R.string.conn_transport_quic
    "punch" -> R.string.conn_transport_punch
    "webrtc" -> R.string.conn_transport_webrtc
    "relay" -> R.string.conn_transport_relay
    "ble" -> R.string.conn_transport_ble
    else -> null
}

private fun natRes(n: String?): Int = when (n) {
    "easy" -> R.string.conn_nat_easy
    "symmetric" -> R.string.conn_nat_symmetric
    else -> R.string.conn_nat_unknown
}

/** One address line: its LAN address and port, IPv6, router mapping, public. */
private fun addrText(a: JSONObject?): String {
    if (a == null) return "–"
    val ip = a.str("ip").orEmpty()
    val port = a.optInt("quic_port")
    val parts = listOf(if (ip.isNotEmpty() && port != 0) "$ip:$port" else ip, a.str("ipv6"), a.str("mapped_quic"), a.str("public"))
    return parts.filterNotNull().filter { it.isNotEmpty() }.distinct().joinToString(", ").ifEmpty { "–" }
}

/** Stops a running test when the screen goes, after the tile's own scope is gone. */
private val background = CoroutineScope(SupervisorJob() + Dispatchers.Main)

/** How this phone reaches computer [node] ([name]): a plain-language line,
 *  the details behind it, recent attempts, and a speed test. */
@Composable
fun ConnectionTile(node: String, name: String) {
    val toast = LocalToast.current
    val scope = rememberCoroutineScope()
    val ctx = LocalContext.current
    var busy by remember { mutableStateOf(false) }
    val report = ConnCheck.reports[node]
    val run = ConnCheck.speed[node]
    val transport = Core.links[node]
    val linked = transport != null
    val bytes = ConnCheck.bytesFor(transport)

    // Leaving the screen stops a test that's running.
    DisposableEffect(node) {
        onDispose {
            if (ConnCheck.speed[node]?.running == true) background.launch { runCatching { ConnCheck.cancelSpeedTest(node) } }
        }
    }

    Tile(title = stringResource(R.string.conn_title)) {
        if (report == null) {
            Muted(stringResource(R.string.conn_hint, name))
        } else {
            Muted(stringResource(R.string.conn_checked, timeAgo(report.at)))
            Report(report.json, name)
        }

        if (run != null && run.running) {
            val phase = when (run.phase) {
                "ping" -> stringResource(R.string.conn_speed_ping)
                "upload" -> stringResource(R.string.conn_speed_upload, name)
                else -> stringResource(R.string.conn_speed_download, name)
            }
            val part = if (run.total > 0) run.done.toFloat() / run.total else 0f
            val fraction = when (run.phase) {
                "ping" -> 0.02f
                "upload" -> 0.05f + part * 0.45f
                else -> 0.5f + part * 0.5f
            }
            Muted(phase, Modifier.announce())
            LinearProgressIndicator(
                progress = { fraction.coerceIn(0f, 1f) },
                modifier = Modifier.fillMaxWidth().progressInfo(stringResource(R.string.a11y_speed_progress), fraction),
                color = T.accent,
                trackColor = T.surface2,
            )
        } else if (run != null) {
            when (run.phase) {
                "done" -> run.result?.let { SpeedResults(it, name) }
                "failed" -> Text(
                    stringResource(R.string.conn_speed_failed, run.error.orEmpty()),
                    Modifier.announce(), style = MaterialTheme.typography.bodySmall, color = T.danger,
                )
                "cancelled" -> Muted(stringResource(R.string.conn_speed_cancelled), Modifier.announce())
            }
        }

        if (!linked) Muted(stringResource(R.string.conn_speed_needs_link, name))
        else if (run?.running != true) Muted(stringResource(R.string.conn_speed_hint, Formatter.formatShortFileSize(ctx, bytes.toLong())))
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            SoftButton(
                stringResource(if (busy) R.string.conn_checking else R.string.conn_check),
                {
                    busy = true
                    scope.launch {
                        try { ConnCheck.diagnose(node) } catch (e: Exception) { toast(friendlyError(e)) } finally { busy = false }
                    }
                },
                enabled = !busy,
            )
            if (run?.running == true) {
                GhostButton(stringResource(R.string.conn_speed_cancel), {
                    scope.launch { runCatching { ConnCheck.cancelSpeedTest(node) } }
                })
            } else if (linked) {
                GhostButton(stringResource(R.string.conn_speed), {
                    scope.launch {
                        try { ConnCheck.startSpeedTest(node, bytes) } catch (e: Exception) { toast(friendlyError(e)) }
                    }
                })
            }
        }
    }
}

@Composable
private fun Report(d: JSONObject, name: String) {
    val link = d.optJSONObject("link")
    val verdict = d.optString("verdict")
    val ago = if (d.isNull("last_try_ago_s")) null else d.optLong("last_try_ago_s")
    val line = verdictRes(verdict)?.let { stringResource(it) } ?: when {
        verdict == "unreachable" && ago != null -> {
            val why = reasonRes(d.str("reason"))
            if (why != null) stringResource(R.string.conn_verdict_unreachable_why, agoText(ago), stringResource(why, name))
            else stringResource(R.string.conn_verdict_unreachable, agoText(ago))
        }
        else -> stringResource(R.string.conn_verdict_not_connected, name)
    }
    val color = when {
        link == null -> T.danger
        !link.optBoolean("direct") -> T.warning
        else -> T.success
    }
    Text(line, Modifier.announce(), style = MaterialTheme.typography.bodyMedium, fontWeight = FontWeight.SemiBold, color = color)

    // What would help.
    val relay = d.optJSONObject("relay")
    if (link == null && verdict == "unreachable") Muted(stringResource(R.string.conn_tip_unreachable, name))
    if (d.optBoolean("both_symmetric")) Muted(stringResource(R.string.conn_tip_symmetric))
    if (relay != null && !relay.optBoolean("signed_in")) Muted(stringResource(R.string.conn_tip_signed_out))
    else if (relay != null && !relay.isNull("reachable") && !relay.optBoolean("reachable")) Muted(stringResource(R.string.conn_tip_relay_down))
    if (link != null && !d.optBoolean("ping_supported")) Muted(stringResource(R.string.conn_tip_old_app, name))

    Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
        if (link != null) {
            val how = transportRes(link.str("transport"))?.let { stringResource(it) } ?: link.optString("transport")
            val remote = link.str("remote")
            Detail(stringResource(R.string.conn_how), if (remote != null) "$how · $remote" else how)
        }
        d.optJSONObject("rtt")?.let { r ->
            val samples = r.optInt("samples")
            val lost = r.optInt("lost")
            var v = if (samples > 0) stringResource(R.string.conn_rtt_value, ms(r.num("avg_ms")), ms(r.num("min_ms")), ms(r.num("max_ms")))
            else stringResource(R.string.conn_rtt_none)
            if (samples > 0 && lost > 0) v += " · " + stringResource(R.string.conn_rtt_lost, lost, lost + samples)
            Detail(stringResource(R.string.conn_rtt), v)
        }
        link?.num("loss_pct")?.let { Detail(stringResource(R.string.conn_loss), stringResource(R.string.conn_loss_value, String.format(java.util.Locale.getDefault(), "%.1f", it))) }
        Detail(name, addrText(d.optJSONObject("peer")))
        Detail(stringResource(R.string.conn_my_address), addrText(d.optJSONObject("this_device")))
        Detail(
            stringResource(R.string.conn_network),
            stringResource(
                R.string.conn_network_value,
                stringResource(natRes(d.optJSONObject("this_device")?.str("nat"))),
                name,
                stringResource(natRes(d.optJSONObject("peer")?.str("nat"))),
            ),
        )
        if (relay != null) {
            val r = when {
                !relay.optBoolean("signed_in") -> R.string.conn_relay_none
                relay.optBoolean("connected") -> R.string.conn_relay_connected
                relay.optBoolean("reachable") -> R.string.conn_relay_ok
                else -> R.string.conn_relay_down
            }
            Detail(stringResource(R.string.conn_relay), stringResource(r))
        }
    }

    val attempts = d.optJSONArray("attempts")
    if (attempts != null && attempts.length() > 0) {
        Text(stringResource(R.string.conn_attempts), style = MaterialTheme.typography.labelMedium, color = T.text2)
        val from = maxOf(0, attempts.length() - 6)
        for (i in attempts.length() - 1 downTo from) {
            val a = attempts.optJSONObject(i) ?: continue
            val stage = a.optString("stage")
            var label = stageRes(stage)?.let { stringResource(it) } ?: stage
            if (stage == "link_up") transportRes(a.str("detail"))?.let { label += " (" + stringResource(it) + ")" }
            Detail(label, agoText(a.optLong("ago_s")))
        }
    }
}

@Composable
private fun SpeedResults(r: JSONObject, name: String) {
    val mbps = { x: Double -> String.format(java.util.Locale.getDefault(), "%.1f", x) }
    Column(verticalArrangement = Arrangement.spacedBy(4.dp), modifier = Modifier.announce()) {
        Detail(stringResource(R.string.conn_speed_up, name), stringResource(R.string.conn_mbps, mbps(r.optDouble("upload_mbps"))))
        Detail(stringResource(R.string.conn_speed_down, name), stringResource(R.string.conn_mbps, mbps(r.optDouble("download_mbps"))))
        val rtt = r.optJSONObject("rtt")
        if (rtt != null && rtt.optInt("samples") > 0) Detail(stringResource(R.string.conn_rtt), "${ms(rtt.num("avg_ms"))} ms")
    }
}

@Composable
private fun Detail(label: String, value: String) {
    Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
        Text(label, style = MaterialTheme.typography.labelMedium, color = T.text2, modifier = Modifier.weight(0.4f))
        Text(value, style = MaterialTheme.typography.bodySmall, color = T.text, modifier = Modifier.weight(0.6f))
    }
}
