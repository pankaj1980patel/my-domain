package com.mydomain.android.ui

import android.text.format.Formatter
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Path
import androidx.compose.ui.graphics.StrokeJoin
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.pluralStringResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import com.mydomain.android.R
import com.mydomain.android.core.BatteryEstimate
import com.mydomain.android.core.BatteryLog
import com.mydomain.android.core.BatterySample
import com.mydomain.android.core.DeviceStatus

/** It sent something [InfoTile] shows. */
fun hasInfo(st: DeviceStatus): Boolean =
    st.battery != null || (st.storageTotal ?: 0L) > 0L || (st.ramTotal ?: 0L) > 0L || st.cpu != null || st.uptimeS != null || st.dnd != null

/** What another device last said about itself: battery, storage, memory,
 *  processor and how long it has been on. A row per field it sent (phones
 *  send no processor use; older versions no uptime). Under the battery bar,
 *  its last 24 hours ([BatteryGraph]). */
@Composable
fun InfoTile(node: String, name: String, st: DeviceStatus) {
    val ctx = LocalContext.current
    val size = { n: Long -> Formatter.formatShortFileSize(ctx, n) }
    Tile(title = stringResource(R.string.info_title)) {
        Muted(stringResource(R.string.info_hint, name))
        st.battery?.let { level ->
            InfoRow(
                stringResource(R.string.info_battery),
                stringResource(if (st.charging == true) R.string.info_battery_charging else R.string.info_battery_level, level),
                level / 100f,
            )
            BatteryLog.samples[node]?.let { BatteryGraph(it) }
        }
        // Disk and memory bars show how full they are.
        listOf(
            Triple(R.string.info_storage, st.storageFree, st.storageTotal),
            Triple(R.string.info_memory, st.ramFree, st.ramTotal),
        ).forEach { (label, free, total) ->
            if (free != null && total != null && total > 0) {
                val left = free.coerceIn(0L, total)
                InfoRow(
                    stringResource(label),
                    stringResource(R.string.info_free, size(left), size(total)),
                    (total - left).toFloat() / total,
                )
            }
        }
        st.cpu?.let { InfoRow(stringResource(R.string.info_cpu), stringResource(R.string.info_cpu_busy, it), it / 100f) }
        st.uptimeS?.let { up ->
            // It answered at `seenAt`: count the time since then too.
            val secs = up + ((System.currentTimeMillis() - st.seenAt) / 1000).coerceAtLeast(0)
            InfoRow(stringResource(R.string.info_uptime), uptimeText(secs))
        }
        st.dnd?.let {
            InfoRow(stringResource(R.string.info_dnd), stringResource(if (it) R.string.info_dnd_on else R.string.info_dnd_off))
        }
    }
}

/** A label and its value on one line, with a bar under them when [fraction] is given. */
@Composable
private fun InfoRow(label: String, value: String, fraction: Float? = null) {
    Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Text(label, style = MaterialTheme.typography.labelMedium, color = T.text2, modifier = Modifier.weight(1f))
            Text(value, style = MaterialTheme.typography.bodyMedium, color = T.text)
        }
        if (fraction != null) {
            LinearProgressIndicator(
                progress = { fraction.coerceIn(0f, 1f) },
                modifier = Modifier.fillMaxWidth(),
                color = T.accent,
                trackColor = T.surface2,
            )
        }
    }
}

/** 300 -> "5 minutes", 12000 -> "3 h 20 min", 190000 -> "2 days, 4 h". */
@Composable
private fun uptimeText(secs: Long): String {
    val mins = (secs / 60).toInt()
    val hours = mins / 60
    val days = hours / 24
    return when {
        days > 0 -> pluralStringResource(R.plurals.info_up_days, days, days, hours % 24)
        hours > 0 -> stringResource(R.string.info_up_hours, hours, mins % 60)
        else -> pluralStringResource(R.plurals.info_up_minutes, mins, mins)
    }
}

private const val DAY_S = 24 * 60 * 60L
/** Further apart than this, the line breaks (the device wasn't heard from). */
private const val GAP_S = 60 * 60L

/** The last 24 hours of its battery: a line over a soft area, charging
 *  stretches shaded, and a guess at how long it lasts (or takes to fill)
 *  when there's enough to go on. Like the desktop's BatteryChart. */
@Composable
private fun BatteryGraph(samples: List<BatterySample>) {
    val now = BatteryLog.nowS()
    val start = now - DAY_S
    val first = samples.indexOfFirst { it.at >= start }
    if (first < 0) return
    // The one before too, so the line comes in from the edge.
    val shown = samples.subList(maxOf(0, first - 1), samples.size)
    if (shown.size < 2) return
    // Runs of samples with no long silence between them.
    val runs = mutableListOf<MutableList<BatterySample>>()
    for (s in shown) {
        val run = runs.lastOrNull()
        if (run != null && s.at - run.last().at <= GAP_S) run.add(s) else runs.add(mutableListOf(s))
    }
    val lineColor = T.accent
    val areaColor = T.accentBg
    val chargingColor = T.successSoft
    val label = stringResource(R.string.info_battery_chart)
    Column(verticalArrangement = Arrangement.spacedBy(2.dp)) {
        Canvas(
            Modifier
                .fillMaxWidth()
                .height(48.dp)
                .clip(RoundedCornerShape(4.dp))
                .background(T.surface2)
                .semantics { contentDescription = label },
        ) {
            val pad = 3.dp.toPx()
            val x = { at: Long -> (at - start).toFloat() / DAY_S * size.width }
            val y = { level: Int -> pad + (1f - level / 100f) * (size.height - 2 * pad) }
            // Where it was charging: from a charging sample to the next one.
            for (run in runs) {
                for (i in 0 until run.size - 1) {
                    if (!run[i].charging) continue
                    val from = x(run[i].at)
                    drawRect(chargingColor, topLeft = Offset(from, 0f), size = Size(x(run[i + 1].at) - from, size.height))
                }
            }
            for (run in runs) {
                val line = Path().apply {
                    run.forEachIndexed { i, s -> if (i == 0) moveTo(x(s.at), y(s.level)) else lineTo(x(s.at), y(s.level)) }
                }
                val area = Path().apply {
                    addPath(line)
                    lineTo(x(run.last().at), size.height)
                    lineTo(x(run.first().at), size.height)
                    close()
                }
                drawPath(area, areaColor)
                drawPath(line, lineColor, style = Stroke(width = 1.5.dp.toPx(), join = StrokeJoin.Round))
            }
        }
        Row {
            Text(stringResource(R.string.info_battery_day_ago), style = MaterialTheme.typography.labelSmall, color = T.text3, modifier = Modifier.weight(1f))
            Text(stringResource(R.string.info_battery_now), style = MaterialTheme.typography.labelSmall, color = T.text3)
        }
        BatteryLog.estimate(samples, now)?.let { Muted(estimateText(it)) }
    }
}

/** "About 6 h left at this rate" / "Full in about 40 min". */
@Composable
private fun estimateText(e: BatteryEstimate): String {
    val mins = e.mins
    val time = when {
        mins < 60 -> stringResource(R.string.info_battery_min, (Math.round(mins / 5) * 5).toInt().coerceIn(5, 55))
        mins < 3 * 60 -> {
            // To the nearest 10 minutes; 1 h 60 min is 2 h.
            val tens = Math.round(mins / 10).toInt()
            val h = tens / 6
            val m = (tens % 6) * 10
            if (m > 0) stringResource(R.string.info_battery_hours_min, h, m) else stringResource(R.string.info_battery_hours, h)
        }
        else -> stringResource(R.string.info_battery_hours, Math.round(mins / 60).toInt())
    }
    return stringResource(if (e.full) R.string.info_battery_full_in else R.string.info_battery_left, time)
}
