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
import androidx.compose.material3.HorizontalDivider
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
import com.mydomain.android.core.TransferBook
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
 * Pause / Resume, Cancel), then the newest finished or failed ones (Open,
 * Retry, Send again), kept across restarts by [TransferLog]. Several files
 * sent together (a folder, or files shared at once) are one row with the
 * overall progress; its files open underneath. [found]: a transfer a search
 * opened, scrolled to and marked.
 */
@Composable
fun TransfersScreen(onOpenDevice: (String) -> Unit, found: String? = null) {
    val all = TransferLog.list
    // A batch's files show inside it.
    val list = all.filter { it.batch == null }
    val filesOf = { id: String -> all.filter { it.batch == id } }
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
            Core.awaitOnScreen()
            now = System.currentTimeMillis()
        }
    }

    LazyColumn(Modifier.fillMaxSize(), state = listState, contentPadding = PaddingValues(start = 16.dp, end = 16.dp, bottom = 16.dp)) {
        // The Files hub's bar and tabs say where you are: the line, and Clear.
        item {
            Row(Modifier.padding(top = 4.dp, bottom = 8.dp), verticalAlignment = Alignment.CenterVertically) {
                Muted(stringResource(R.string.transfers_subtitle), Modifier.weight(1f))
                if (finished.isNotEmpty()) {
                    GhostButton(stringResource(R.string.transfers_clear), { TransferLog.clearFinished() }, small = true)
                }
            }
        }
        if (list.isEmpty()) {
            item { EmptyState(stringResource(R.string.transfers_empty_title), stringResource(R.string.transfers_empty_body)) }
        }
        if (active.isNotEmpty()) {
            item { Section(stringResource(R.string.transfers_active)) }
            items(active, key = { "a-" + it.id }) { Box(Modifier.foundMark(it.id == found)) { ActiveRow(it, now, filesOf(it.id)) } }
        }
        if (finished.isNotEmpty()) {
            item { Section(stringResource(R.string.transfers_recent)) }
            items(finished, key = { "f-" + it.id }) { Box(Modifier.foundMark(it.id == found)) { FinishedRow(it, now, onOpenDevice, filesOf(it.id)) } }
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
            Text(if (t.kind == "folder") "▤" else if (t.incoming) "↓" else "↑", color = T.text, style = MaterialTheme.typography.bodyMedium)
        }
        Spacer(Modifier.width(12.dp))
        Column(Modifier.weight(1f)) {
            val name = if (t.isBatch && t.kind != "folder" && t.count > 1) {
                t.name + " · " + stringResource(R.string.transfer_batch_files, t.count)
            } else t.name
            Text(
                name, style = MaterialTheme.typography.bodyMedium, color = T.text,
                maxLines = 1, overflow = TextOverflow.Ellipsis,
            )
            val link = t.link
            Muted(
                if (link != null) dlHeadLine(t.incoming, t.node, link)
                else stringResource(if (t.incoming) R.string.transfers_from else R.string.transfers_to, device),
            )
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

/** A batch's files, opened under its row. */
@Composable
private fun BatchFiles(files: List<TransferRecord>) {
    val ctx = LocalContext.current
    Column(Modifier.fillMaxWidth().padding(start = 40.dp), verticalArrangement = Arrangement.spacedBy(2.dp)) {
        files.take(TransferBook.MAX_BATCH_FILES).forEach { f ->
            Row(Modifier.semantics(mergeDescendants = true) {}) {
                Text(
                    f.name, Modifier.weight(1f), style = MaterialTheme.typography.labelMedium,
                    color = if (f.state == TransferState.Failed) T.danger else T.text2,
                    maxLines = 1, overflow = TextOverflow.Ellipsis,
                )
                Spacer(Modifier.width(8.dp))
                Muted(
                    when (f.state) {
                        TransferState.Done -> Formatter.formatShortFileSize(ctx, f.total)
                        TransferState.Failed -> f.reason ?: ""
                        TransferState.Active -> "${(f.fraction * 100).toInt()}%"
                    },
                )
            }
        }
    }
}

/** Show files / Hide files, for a batch with files. */
@Composable
private fun FilesToggle(open: Boolean, onToggle: () -> Unit) {
    GhostButton(stringResource(if (open) R.string.transfer_hide_files else R.string.transfer_show_files), onToggle, small = true)
}

@Composable
private fun ActiveRow(t: TransferRecord, now: Long, files: List<TransferRecord> = emptyList()) {
    val ctx = LocalContext.current
    val toast = LocalToast.current
    val size = { n: Long -> Formatter.formatShortFileSize(ctx, n) }
    val moving = !t.paused && t.rate > 0 && now - t.movedAt <= STALE_MS
    var open by remember { mutableStateOf(false) }
    val dlLine = if (t.link != null) dlActiveLine(t.id) else null
    val parts = buildList {
        // A link download that isn't moving yet says why (asking, queued...).
        if (dlLine != null && t.done == 0L) {
            add(dlLine)
        } else if (t.total == 0L && t.link != null) {
            add(stringResource(R.string.transfers_progress_unknown, size(t.done)))
        } else if (t.total == 0L) {
            add(stringResource(R.string.transfers_waiting, Core.peerName(t.node)))
        } else {
            if (t.isBatch) add(stringResource(R.string.transfer_batch_progress, t.filesDone, t.count))
            add(stringResource(R.string.transfers_progress, size(t.done), size(t.total)))
            if (t.paused) add(stringResource(R.string.transfer_paused))
            if (moving) {
                add(stringResource(R.string.transfers_speed, size(Math.round(t.rate))))
                t.secondsLeft()?.let { add(stringResource(R.string.transfers_eta, duration(it))) }
            }
        }
    }
    Column(Modifier.fillMaxWidth().padding(vertical = 10.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
        Head(t) {
            Row(horizontalArrangement = Arrangement.spacedBy(4.dp)) {
                if (t.isBatch && files.isNotEmpty()) FilesToggle(open) { open = !open }
                if (t.link == null) {
                    GhostButton(
                        stringResource(if (t.paused) R.string.transfer_resume else R.string.transfer_pause),
                        { Core.pauseFileAsync(t.id, !t.paused) { toast(friendlyError(it)) } },
                        small = true,
                    )
                }
                GhostButton(stringResource(R.string.cancel), { Core.cancelFileAsync(t.id) }, small = true)
            }
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
            t.via?.let { Muted(" · " + stringResource(viaRes(it))) }
        }
        if (open) BatchFiles(files)
    }
    HorizontalDivider(color = T.border)
}

/** "via Wi-Fi Direct" / "via phone hotspot". */
private fun viaRes(via: String): Int =
    if (via == TransferBook.VIA_HOTSPOT) R.string.transfers_via_hotspot else R.string.transfers_via_wifi_direct

/** Whether [uri] (a picked file) can still be read. Blocking. */
private fun readable(ctx: Context, uri: String): Boolean = runCatching {
    ctx.contentResolver.query(Uri.parse(uri), arrayOf(OpenableColumns.DISPLAY_NAME), null, null, null)?.use { it.moveToFirst() } == true &&
        ctx.contentResolver.openFileDescriptor(Uri.parse(uri), "r")?.use { true } == true
}.getOrDefault(false)

@Composable
private fun FinishedRow(t: TransferRecord, now: Long, onOpenDevice: (String) -> Unit, files: List<TransferRecord> = emptyList()) {
    val ctx = LocalContext.current
    val toast = LocalToast.current
    val size = Formatter.formatShortFileSize(ctx, t.total)
    // [now] ticks, so "2 min ago" keeps up.
    val whenText = timeAgo(t.ended ?: t.started, now)
    val via = t.via?.let { " · " + stringResource(viaRes(it)) } ?: ""
    val counted = if (t.isBatch) stringResource(R.string.transfer_batch_files, t.count) + " · " else ""
    val failedFiles = if (t.isBatch && t.filesFailed > 0) stringResource(R.string.transfer_batch_failed, t.filesFailed) + " · " else ""
    val line = when {
        t.state == TransferState.Done ->
            counted + stringResource(if (t.incoming) R.string.transfers_done_saved else R.string.transfers_done_sent, size, whenText) + via
        t.isBatch && t.reason != "cancelled" -> failedFiles + stringResource(R.string.transfers_failed, t.reason ?: "", whenText)
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
    var open by remember { mutableStateOf(false) }
    val retrying = stringResource(R.string.transfer_retrying, t.name)
    Column(Modifier.fillMaxWidth().padding(vertical = 10.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
        Head(t) {
            if (t.isBatch && files.isNotEmpty()) FilesToggle(open) { open = !open }
            when {
                // Sent (or asked for) from here and failed: the core sends
                // it again (a batch's files that didn't go, a file, a link).
                !t.incoming && t.state == TransferState.Failed && (t.isBatch || t.link != null || !canResend) ->
                    SoftButton(stringResource(R.string.transfer_retry), {
                        if (sending) return@SoftButton
                        sending = true
                        Core.retryFileAsync(
                            t.id,
                            onDone = { sending = false; toast(retrying) },
                            onError = { sending = false; toast(friendlyError(it)) },
                        )
                    }, enabled = !sending)
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
        if (open) BatchFiles(files)
    }
    HorizontalDivider(color = T.border)
}
