package com.mydomain.android.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import com.mydomain.android.R
import com.mydomain.android.core.Core
import com.mydomain.android.core.Tone
import com.mydomain.android.core.TransferLog
import com.mydomain.android.core.timeAgo

/** What happened, in plain sentences; the way to Transfers on top. */
@Composable
fun ActivityScreen(onTransfers: () -> Unit) {
    LazyColumn(Modifier.fillMaxSize(), contentPadding = PaddingValues(16.dp)) {
        item { PageHead(stringResource(R.string.activity_title), stringResource(R.string.activity_subtitle)) }
        item {
            val active = TransferLog.list.count { !it.finished }
            Tile(Modifier.padding(vertical = 8.dp), onClick = onTransfers) {
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Text(
                        if (active > 0) stringResource(R.string.transfers_open_active, active)
                        else stringResource(R.string.transfers_open),
                        Modifier.weight(1f), style = MaterialTheme.typography.titleMedium, color = T.text,
                    )
                    Text("›", style = MaterialTheme.typography.titleMedium, color = T.text3)
                }
                Muted(stringResource(R.string.transfers_subtitle))
            }
        }
        if (Core.activity.isEmpty()) {
            item { EmptyState(stringResource(R.string.activity_empty_title), stringResource(R.string.activity_empty_body)) }
        }
        items(Core.activity, key = { it.id }) { a ->
            val dot = when (a.tone) {
                Tone.Ok -> T.success
                Tone.Warn -> T.warning
                Tone.Error -> T.danger
                Tone.Info -> T.text3
            }
            Row(Modifier.fillMaxWidth().padding(vertical = 12.dp), verticalAlignment = Alignment.Top) {
                Box(Modifier.padding(top = 6.dp).size(8.dp).background(dot, CircleShape))
                Spacer(Modifier.width(12.dp))
                Text(a.text, Modifier.weight(1f), style = MaterialTheme.typography.bodyMedium, color = T.text)
                Spacer(Modifier.width(12.dp))
                Muted(timeAgo(a.ts))
            }
            HorizontalDivider(color = T.border)
        }
    }
}
