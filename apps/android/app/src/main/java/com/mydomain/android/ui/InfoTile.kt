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
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.pluralStringResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import com.mydomain.android.R
import com.mydomain.android.core.DeviceStatus

/** It sent something [InfoTile] shows. */
fun hasInfo(st: DeviceStatus): Boolean =
    st.battery != null || (st.storageTotal ?: 0L) > 0L || (st.ramTotal ?: 0L) > 0L || st.cpu != null || st.uptimeS != null

/** What another device last said about itself: battery, storage, memory,
 *  processor and how long it has been on. A row per field it sent (phones
 *  send no processor use; older versions no uptime). */
@Composable
fun InfoTile(name: String, st: DeviceStatus) {
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
