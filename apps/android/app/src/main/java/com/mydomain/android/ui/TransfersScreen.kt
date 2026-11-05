package com.mydomain.android.ui

import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.semantics.heading
import android.content.Context
import android.net.Uri
import android.provider.OpenableColumns
import android.text.format.Formatter
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
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
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableLongStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import com.mydomain.android.FileActions
import com.mydomain.android.R
import com.mydomain.android.core.Core
import com.mydomain.android.core.TransferLog
import com.mydomain.android.core.TransferRecord
import com.mydomain.android.core.TransferState
import com.mydomain.android.core.friendlyError
import com.mydomain.android.core.timeAgo
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.withContext

/** Speed older than this (no bytes moved) isn't shown. */
private const val STALE_MS = 5_000L

/**
 * Transfers (pushed from the Activity tab): every file going to or coming
 * from your devices. Those under way first (progress, speed, time left,
 * Cancel), then the newest finished or failed ones (Open, Send again),
 * kept across restarts by [TransferLog]. [found]: a transfer a search
 * opened, scrolled to and marked.
 */
@Composable
fun TransfersScreen(onBack: () -> Unit, onOpenDevice: (String) -> Unit, found: String? = null) {
    val list = TransferLog.list
    val active = list.filter { !it.finished }
    val finished = list.filter { it.finished }
    val listState = rememberLazyListState()
    LaunchedEffect(found) {
        val t = found?.let { id -> list.firstOrNull { it.id == id } } ?: return@LaunchedEffect
        listState.scrollToKey((if (t.finished) "f-" else "a-") + t.id)
    }
    // A clock for stale speeds and "2 min ago".
    var now by remember { mutableLongStateOf(System.currentTimeMillis()) }
    LaunchedEffect(Unit) {
        while (true) {
            delay(1000)
            now = System.currentTimeMillis()
        }
    }

    LazyColumn(Modifier.fillMaxSize(), state = listState, contentPadding = PaddingValues(start = 16.dp, end = 16.dp, bottom = 16.dp)) {
        item {
            Row(Modifier.padding(top = 4.dp), verticalAlignment = Alignment.CenterVertically) {
                IconButton(onClick = onBack) {
                    Icon(Icons.AutoMirrored.Filled.ArrowBack, stringResource(R.string.back), tint = T.text)
                }
                Column(Modifier.weight(1f)) {
                    Text(
                        stringResource(R.string.transfers_title), Modifier.semantics { heading() },
                        style = MaterialTheme.typography.headlineSmall, color = T.text,
                    )
                }
                if (finished.isNotEmpty()) {
                    GhostButton(stringResource(R.string.transfers_clear), { TransferLog.clearFinished() }, small = true)
                }
            }
            Muted(stringResource(R.string.transfers_subtitle), Modifier.padding(bottom = 8.dp))
        }
        if (list.isEmpty()) {
            item { EmptyState(stringResource(R.string.transfers_empty_title), stringResource(R.string.transfers_empty_body)) }
        }
        if (active.isNotEmpty()) {
            item { Section(stringResource(R.string.transfers_active)) }
            items(active, key = { "a-" + it.id }) { Box(Modifier.foundMark(it.id == found)) { ActiveRow(it, now) } }
        }
        if (finished.isNotEmpty()) {
            item { Section(stringResource(R.string.transfers_recent)) }
            items(finished, key = { "f-" + it.id }) { Box(Modifier.foundMark(it.id == found)) { FinishedRow(it, now, onOpenDevice) } }
        }
    }
}

@Composable
private fun Section(title: String) {
    Text(
        title, Modifier.padding(top = 16.dp, bottom = 4.dp).semantics { heading() },
        style = MaterialTheme.typography.titleSmall, color = T.text2,
    )
}

/** ↑ sent / ↓ received, the name and the device; [action] at the end. */
@Composable
private fun Head(t: TransferRecord, action: @Composable () -> Unit) {
    val device = Core.peerName(t.node)
    val dir = stringResource(if (t.incoming) R.string.transfers_dir_received else R.string.transfers_dir_sent)
    // "Sent, report.pdf, to Pixel" as one stop; the button stays its own.
    Row(Modifier.semantics(mergeDescendants = true) {}, verticalAlignment = Alignment.CenterVertically) {
        Box(
            Modifier.size(28.dp).background(T.surface2, CircleShape).clearAndSetSemantics { contentDescription = dir },
            contentAlignment = Alignment.Center,
        ) {
            Text(if (t.incoming) "↓" else "↑", color = T.text, style = MaterialTheme.typography.bodyMedium)
        }
        Spacer(Modifier.width(12.dp))
        Column(Modifier.weight(1f)) {
            Text(
                t.name, style = MaterialTheme.typography.bodyMedium, color = T.text,
                maxLines = 1, overflow = TextOverflow.Ellipsis,
            )
            Muted(stringResource(if (t.incoming) R.string.transfers_from else R.string.transfers_to, device))
        }
        Spacer(Modifier.width(8.dp))
        action()
    }
}

/** 75 -> "1 min", for time left. */
@Composable
private fun duration(secs: Long): String {
    val s = secs.coerceAtLeast(1)
    if (s < 60) return stringResource(R.string.transfers_secs, s.toInt())
    val m = Math.round(s / 60.0).toInt()
    if (m < 60) return stringResource(R.string.transfers_mins, m)
    return stringResource(R.string.transfers_hours, m / 60, m % 60)
}

@Composable
private fun ActiveRow(t: TransferRecord, now: Long) {
    val ctx = LocalContext.current
    val size = { n: Long -> Formatter.formatShortFileSize(ctx, n) }
    val moving = t.rate > 0 && now - t.movedAt <= STALE_MS
    val parts = buildList {
        if (t.total == 0L) {
            add(stringResource(R.string.transfers_waiting, Core.peerName(t.node)))
        } else {
            add(stringResource(R.string.transfers_progress, size(t.done), size(t.total)))
            if (moving) {
                add(stringResource(R.string.transfers_speed, size(Math.round(t.rate))))
                t.secondsLeft()?.let { add(stringResource(R.string.transfers_eta, duration(it))) }
            }
        }
    }
    Column(Modifier.fillMaxWidth().padding(vertical = 10.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
        Head(t) {
            GhostButton(stringResource(R.string.cancel), { Core.cancelFileAsync(t.id) }, small = true)
        }
        LinearProgressIndicator(
            progress = { t.fraction },
            modifier = Modifier.fillMaxWidth().progressInfo(t.name, t.fraction),
            color = T.accent,
            trackColor = T.surface2,
        )
        Row {
            Muted(parts.joinToString(" · "), Modifier.weight(1f))
            val from = t.resumedFrom
            if (from != null && t.total > 0) {
                Muted(stringResource(R.string.transfers_resumed, (from * 100 / t.total).toInt()))
            }
        }
    }
    HorizontalDivider(color = T.border)
}

/** Whether [uri] (a picked file) can still be read. Blocking. */
private fun readable(ctx: Context, uri: String): Boolean = runCatching {
    ctx.contentResolver.query(Uri.parse(uri), arrayOf(OpenableColumns.DISPLAY_NAME), null, null, null)?.use { it.moveToFirst() } == true &&
        ctx.contentResolver.openFileDescriptor(Uri.parse(uri), "r")?.use { true } == true
}.getOrDefault(false)

@Composable
private fun FinishedRow(t: TransferRecord, now: Long, onOpenDevice: (String) -> Unit) {
    val ctx = LocalContext.current
    val toast = LocalToast.current
    val size = Formatter.formatShortFileSize(ctx, t.total)
    // [now] ticks, so "2 min ago" keeps up.
    val whenText = timeAgo(t.ended ?: t.started, now)
    val line = when {
        t.state == TransferState.Done ->
            stringResource(if (t.incoming) R.string.transfers_done_saved else R.string.transfers_done_sent, size, whenText)
        t.reason == "cancelled" -> stringResource(R.string.transfers_cancelled, whenText)
        else -> stringResource(R.string.transfers_failed, t.reason ?: "", whenText)
    }
    val noApp = stringResource(R.string.file_no_app)
    val gone = stringResource(R.string.transfers_source_gone)
    val again = stringResource(R.string.transfers_sending_again, t.name, Core.peerName(t.node))
    // A sent file's picked original, if it can still be read (it may be
    // gone, or its permission ended with the app).
    val source = t.source
    val canResend by produceState(false, source) {
        value = source != null && withContext(Dispatchers.IO) { readable(ctx, source) }
    }
    var sending by remember { mutableStateOf(false) }
    Column(Modifier.fillMaxWidth().padding(vertical = 10.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
        Head(t) {
            when {
                t.incoming && t.state == TransferState.Done && t.uri != null ->
                    GhostButton(stringResource(R.string.file_open), {
                        if (!FileActions.open(ctx, t.uri, t.mime)) toast(noApp)
                    }, small = true)
                t.incoming && t.state == TransferState.Done ->
                    GhostButton(stringResource(R.string.transfers_show), { onOpenDevice(t.node) }, small = true)
                !t.incoming && canResend && source != null ->
                    SoftButton(stringResource(R.string.transfers_send_again), {
                        if (sending) return@SoftButton
                        sending = true
                        toast(again)
                        Core.sendFiles(
                            t.node, listOf(Uri.parse(source)),
                            onDone = { sending = false },
                            onError = { toast(if (it is SecurityException || it is java.io.FileNotFoundException) gone else friendlyError(it)) },
                        )
                    }, enabled = !sending)
            }
        }
        Text(
            line, style = MaterialTheme.typography.labelMedium,
            color = if (t.state == TransferState.Failed && t.reason != "cancelled") T.danger else T.text2,
        )
    }
    HorizontalDivider(color = T.border)
}
