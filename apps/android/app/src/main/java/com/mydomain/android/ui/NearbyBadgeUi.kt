package com.mydomain.android.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import com.mydomain.android.ComputerAudio
import com.mydomain.android.R
import com.mydomain.android.core.Core
import com.mydomain.android.core.NearbyBadge

/** "Nearby · low latency" while a stream with [node] runs on a local link
 *  (nearby media mode), with its latency ("12 ms, was 140 ms"). Nothing
 *  otherwise. */
@Composable
fun NearbyMediaBadge(node: String) {
    val s = Core.nearby[node]
    val latency = NearbyBadge.latency(s)
    val show = NearbyBadge.show(s)
    if (!show && latency == null) return
    Row(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.CenterVertically) {
        if (show) NearbyPill()
        when (latency) {
            is NearbyBadge.Latency.Was -> Muted(stringResource(R.string.nearby_latency_was, latency.ms, latency.beforeMs))
            is NearbyBadge.Latency.Now -> Muted(stringResource(R.string.nearby_latency, latency.ms))
            null -> {}
        }
    }
}

/** The badge itself (also on the connection map's details). */
@Composable
fun NearbyPill(modifier: Modifier = Modifier) {
    Text(
        stringResource(R.string.nearby_badge),
        modifier = modifier
            .background(T.successSoft, RoundedCornerShape(50))
            .padding(horizontal = 8.dp, vertical = 2.dp),
        style = MaterialTheme.typography.labelSmall,
        color = T.success,
    )
}

/** The computer's sound playing here: the buffer, what was made up or
 *  recovered, and the output's own underruns (for "it stutters"). */
@Composable
fun ComputerAudioDetails() {
    val s = ComputerAudio.stats ?: return
    Muted(stringResource(R.string.caudio_stats_buffer, s.bufferedMs, s.targetMs))
    Muted(
        stringResource(
            R.string.caudio_stats_loss,
            s.underruns, s.concealedMs, s.recovered.toInt(), s.outputUnderruns,
        ),
    )
}
