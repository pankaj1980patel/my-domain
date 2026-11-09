package com.mydomain.android.ui

import android.graphics.BitmapFactory
import android.text.format.Formatter
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import com.mydomain.android.R
import com.mydomain.android.core.ClipBook
import com.mydomain.android.core.ClipEntry
import com.mydomain.android.core.ClipLog
import com.mydomain.android.core.ClipPage
import com.mydomain.android.core.ClipPicture
import com.mydomain.android.core.ClipRecord
import com.mydomain.android.core.Core
import com.mydomain.android.core.friendlyError
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import java.io.File
import java.text.DateFormat
import java.util.Calendar
import java.util.Date

/** Rows shown at first, and added by "Show more". */
private const val PAGE = 30

/** The longer side a picture is shown at, at most, when opened. */
private const val PREVIEW_MAX = 2048

/** The time, with the date when it wasn't today. */
private fun whenOf(ts: Long): String {
    val d = Date(ts)
    val clock = DateFormat.getTimeInstance(DateFormat.SHORT).format(d)
    val then = Calendar.getInstance().apply { time = d }
    val now = Calendar.getInstance()
    val today = then.get(Calendar.YEAR) == now.get(Calendar.YEAR) && then.get(Calendar.DAY_OF_YEAR) == now.get(Calendar.DAY_OF_YEAR)
    return if (today) clock else "${DateFormat.getDateInstance(DateFormat.MEDIUM).format(d)} $clock"
}

/** [file] decoded off the main thread, at most [max] px along its longer side
 *  (roughly: a power-of-two step); null while loading or when unreadable. */
@Composable
private fun rememberPicture(file: File?, max: Int): ImageBitmap? {
    val state = produceState<ImageBitmap?>(null, file?.path, max) {
        value = withContext(Dispatchers.IO) {
            runCatching {
                val f = file?.takeIf { it.isFile } ?: return@runCatching null
                val bounds = BitmapFactory.Options().apply { inJustDecodeBounds = true }
                BitmapFactory.decodeFile(f.path, bounds)
                val opts = BitmapFactory.Options().apply { inSampleSize = ClipBook.sampleSize(bounds.outWidth, bounds.outHeight, max) }
                BitmapFactory.decodeFile(f.path, opts)?.asImageBitmap()
            }.getOrNull()
        }
    }
    return state.value
}

/**
 * Clipboard history for one device: the text and pictures sent to it and
 * received from it as clipboard, kept on this phone ([ClipLog], the last
 * 200; pictures the newest 50), searchable. "Copy" puts a clip on this
 * phone's clipboard; "Send again" puts it on the device's. A picture also
 * has "Save" (to Pictures/my-domain) and "Open" (larger, here). Like the
 * desktop's ClipHistoryTile. [found]: a clip a search opened: the list shows
 * down to it, and it's marked and brought into view.
 */
@Composable
fun ClipHistoryTile(node: String, name: String, found: Long? = null) {
    val toast = LocalToast.current
    var query by remember(node) { mutableStateOf("") }
    // Far enough down to show the found clip, in whole pages.
    val start = remember(node, found) {
        val at = if (found == null) -1 else ClipLog.list.filter { it.node == node }.indexOfFirst { it.id == found }
        if (at < 0) PAGE else (at / PAGE + 1) * PAGE
    }
    // A new search starts from the top again.
    var limit by remember(node, query) { mutableIntStateOf(if (query.isEmpty()) start else PAGE) }
    // "Clear" waiting on its Undo: shown as cleared already.
    val clearKey = "clips:$node"
    val page = if (PendingClears.pending(clearKey)) ClipPage(emptyList(), 0, 0) else ClipLog.page(node, query, limit)
    val undo = LocalUndo.current
    val undoLabel = stringResource(R.string.notes_undo)
    var opened by remember(node) { mutableStateOf<ClipRecord?>(null) }

    val cleared = stringResource(R.string.clip_history_cleared)

    Tile {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Text(
                stringResource(R.string.clip_history_title), style = MaterialTheme.typography.titleMedium, color = T.text,
                modifier = Modifier.weight(1f).semantics { heading() },
            )
            if (page.kept > 0) {
                GhostButton(stringResource(R.string.clip_history_clear), {
                    PendingClears.ask(undo, clearKey, cleared, undoLabel) { ClipLog.clear(node) }
                }, small = true)
            }
        }
        if (page.kept == 0) {
            Muted(stringResource(R.string.clip_history_empty, name))
            return@Tile
        }
        Field(query, { query = it }, stringResource(R.string.clip_history_search))
        if (page.total == 0) {
            Muted(stringResource(R.string.clip_history_no_match))
            return@Tile
        }
        page.entries.forEach { e ->
            val pic = e.picture
            if (pic == null) TextRow(e, node, name, found) else PictureRow(e, pic, node, name, found) { opened = e }
        }
        if (page.total > page.entries.size) {
            Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
                Muted(stringResource(R.string.clip_history_showing, page.entries.size, page.total), Modifier.weight(1f))
                SoftButton(stringResource(R.string.clip_history_more), { limit += PAGE })
            }
        }
    }
    opened?.let { e -> e.picture?.let { PicturePreview(e, it, name) { opened = null } } }
}

/** The way it went: an arrow, read out as "From …" / "Sent to …". */
@Composable
private fun WayArrow(e: ClipRecord, name: String) {
    val way = stringResource(if (e.incoming) R.string.clip_history_from else R.string.clip_history_to, name)
    Text(
        if (e.incoming) "↓" else "↑",
        style = MaterialTheme.typography.titleMedium,
        color = if (e.incoming) T.accentFg else T.text2,
        modifier = Modifier.clearAndSetSemantics { contentDescription = way },
    )
}

@Composable
private fun TextRow(e: ClipRecord, node: String, name: String, found: Long?) {
    val toast = LocalToast.current
    val scope = rememberCoroutineScope()
    val copied = stringResource(R.string.clip_history_copied)
    val sent = stringResource(R.string.clip_history_sent, name)
    val queued = stringResource(R.string.clip_history_queued, name)
    // Way, text and time as one stop; Copy and Send again stay their own.
    Row(Modifier.foundMark(e.id == found).semantics(mergeDescendants = true) {}, verticalAlignment = Alignment.Top) {
        WayArrow(e, name)
        Spacer(Modifier.width(10.dp))
        Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(4.dp)) {
            Text(e.text, style = MaterialTheme.typography.bodyMedium, color = T.text, maxLines = 3, overflow = TextOverflow.Ellipsis)
            val note = if (e.truncated) "${whenOf(e.at)} · ${stringResource(R.string.clip_history_truncated)}" else whenOf(e.at)
            Muted(note)
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                GhostButton(stringResource(R.string.clip_history_copy), {
                    runCatching { Core.copyFromHistory(ClipEntry(e.text, if (e.incoming) node else null, System.currentTimeMillis())) }
                        .onSuccess { toast(copied) }
                        .onFailure { toast(friendlyError(it)) }
                }, small = true)
                GhostButton(stringResource(R.string.clip_history_send_again), {
                    scope.launch {
                        try {
                            val how = Core.sendText(node, e.text, "clipboard")
                            toast(if (how == "queued") queued else sent)
                        } catch (ex: Exception) {
                            toast(friendlyError(ex))
                        }
                    }
                }, small = true)
            }
        }
    }
}

/** "800 × 600 · 1.2 MB". */
@Composable
private fun detailsOf(p: ClipPicture): String =
    stringResource(R.string.clip_history_picture_details, p.width, p.height, Formatter.formatShortFileSize(LocalContext.current, p.bytes))

@OptIn(ExperimentalLayoutApi::class)
@Composable
private fun PictureRow(e: ClipRecord, p: ClipPicture, node: String, name: String, found: Long?, open: () -> Unit) {
    val ctx = LocalContext.current
    val toast = LocalToast.current
    val scope = rememberCoroutineScope()
    val copied = stringResource(R.string.clip_history_picture_copied)
    val sending = stringResource(R.string.clip_history_picture_sending, name)
    val saved = stringResource(R.string.clip_history_picture_saved)
    val needsQ = stringResource(R.string.clip_history_save_needs_q)
    val details = detailsOf(p)
    val alt = stringResource(R.string.clip_history_picture_alt, details)
    val thumb = rememberPicture(ClipLog.thumbFile(p), ClipBook.THUMB_MAX)
    Row(Modifier.foundMark(e.id == found), verticalAlignment = Alignment.Top) {
        WayArrow(e, name)
        Spacer(Modifier.width(10.dp))
        Box(
            Modifier.size(64.dp).clip(RoundedCornerShape(8.dp)).background(T.surface2).clickable(onClick = open)
                .semantics { contentDescription = alt },
            contentAlignment = Alignment.Center,
        ) {
            if (thumb != null) Image(thumb, null, Modifier.size(64.dp), contentScale = ContentScale.Crop)
            else Text("🖼", style = MaterialTheme.typography.titleLarge)
        }
        Spacer(Modifier.width(10.dp))
        Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(4.dp)) {
            Column(Modifier.semantics(mergeDescendants = true) {}) {
                Text(stringResource(R.string.clip_history_picture), style = MaterialTheme.typography.bodyMedium, color = T.text)
                Muted(details)
                Muted(whenOf(e.at))
            }
            FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalArrangement = Arrangement.spacedBy(4.dp)) {
                GhostButton(stringResource(R.string.clip_history_copy), {
                    runCatching { Core.copyKeptPicture(ctx, p) }
                        .onSuccess { toast(copied) }
                        .onFailure { toast(friendlyError(it)) }
                }, small = true)
                GhostButton(stringResource(R.string.clip_history_send_again), {
                    scope.launch {
                        try {
                            Core.sendKeptPicture(ctx, node, p)
                            toast(sending)
                        } catch (ex: Exception) {
                            toast(friendlyError(ex))
                        }
                    }
                }, small = true)
                GhostButton(stringResource(R.string.clip_history_save), {
                    scope.launch {
                        try {
                            toast(if (Core.saveKeptPicture(ctx, p)) saved else needsQ)
                        } catch (ex: Exception) {
                            toast(friendlyError(ex))
                        }
                    }
                }, small = true)
                GhostButton(stringResource(R.string.clip_history_open), open, small = true)
            }
        }
    }
}

/** A kept picture shown larger, with the way it went, its size and time. */
@Composable
private fun PicturePreview(e: ClipRecord, p: ClipPicture, name: String, close: () -> Unit) {
    val details = detailsOf(p)
    val full = rememberPicture(ClipLog.pictureFile(p), PREVIEW_MAX)
    val way = stringResource(if (e.incoming) R.string.clip_history_from else R.string.clip_history_to, name)
    // Gone from the history meanwhile (pushed out, cleared): nothing to show.
    LaunchedEffect(ClipLog.list) { if (ClipLog.list.none { it.id == e.id }) close() }
    AlertDialog(
        onDismissRequest = close,
        containerColor = T.bar,
        title = { Text(stringResource(R.string.clip_history_picture), color = T.text) },
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                Box(Modifier.fillMaxWidth().heightIn(min = 120.dp, max = 480.dp), contentAlignment = Alignment.Center) {
                    if (full != null) {
                        Image(
                            full, stringResource(R.string.clip_history_picture_alt, details),
                            Modifier.fillMaxWidth(), contentScale = ContentScale.Fit,
                        )
                    }
                }
                Muted("$way · $details · ${whenOf(e.at)}")
            }
        },
        confirmButton = {
            TextButton(close) { Text(stringResource(R.string.clip_history_close), color = T.text2) }
        },
    )
}
