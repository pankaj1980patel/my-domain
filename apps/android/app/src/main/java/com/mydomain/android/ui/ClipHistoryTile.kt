package com.mydomain.android.ui

import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.semantics.heading
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.width
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import com.mydomain.android.R
import com.mydomain.android.core.ClipEntry
import com.mydomain.android.core.ClipLog
import com.mydomain.android.core.Core
import com.mydomain.android.core.friendlyError
import kotlinx.coroutines.launch
import java.text.DateFormat
import java.util.Calendar
import java.util.Date

/** Rows shown at first, and added by "Show more". */
private const val PAGE = 30

/** The time, with the date when it wasn't today. */
private fun whenOf(ts: Long): String {
    val d = Date(ts)
    val clock = DateFormat.getTimeInstance(DateFormat.SHORT).format(d)
    val then = Calendar.getInstance().apply { time = d }
    val now = Calendar.getInstance()
    val today = then.get(Calendar.YEAR) == now.get(Calendar.YEAR) && then.get(Calendar.DAY_OF_YEAR) == now.get(Calendar.DAY_OF_YEAR)
    return if (today) clock else "${DateFormat.getDateInstance(DateFormat.MEDIUM).format(d)} $clock"
}

/**
 * Clipboard history for one device: the text sent to it and received from
 * it as clipboard, kept on this phone ([ClipLog], the last 200), searchable.
 * "Copy" puts a clip on this phone's clipboard; "Send again" puts it on the
 * device's. Like the desktop's ClipHistoryTile. [found]: a clip a search
 * opened: the list shows down to it, and it's marked and brought into view.
 */
@Composable
fun ClipHistoryTile(node: String, name: String, found: Long? = null) {
    val toast = LocalToast.current
    val scope = rememberCoroutineScope()
    var query by remember(node) { mutableStateOf("") }
    // Far enough down to show the found clip, in whole pages.
    val start = remember(node, found) {
        val at = if (found == null) -1 else ClipLog.list.filter { it.node == node }.indexOfFirst { it.id == found }
        if (at < 0) PAGE else (at / PAGE + 1) * PAGE
    }
    // A new search starts from the top again.
    var limit by remember(node, query) { mutableIntStateOf(if (query.isEmpty()) start else PAGE) }
    val page = ClipLog.page(node, query, limit)

    val copied = stringResource(R.string.clip_history_copied)
    val cleared = stringResource(R.string.clip_history_cleared)
    val sent = stringResource(R.string.clip_history_sent, name)
    val queued = stringResource(R.string.clip_history_queued, name)
    val from = stringResource(R.string.clip_history_from, name)
    val to = stringResource(R.string.clip_history_to, name)

    Tile {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Text(
                stringResource(R.string.clip_history_title), style = MaterialTheme.typography.titleMedium, color = T.text,
                modifier = Modifier.weight(1f).semantics { heading() },
            )
            if (page.kept > 0) {
                GhostButton(stringResource(R.string.clip_history_clear), {
                    ClipLog.clear(node)
                    toast(cleared)
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
            // Way, text and time as one stop; Copy and Send again stay their own.
            Row(Modifier.foundMark(e.id == found).semantics(mergeDescendants = true) {}, verticalAlignment = Alignment.Top) {
                val way = if (e.incoming) from else to
                Text(
                    if (e.incoming) "↓" else "↑",
                    style = MaterialTheme.typography.titleMedium,
                    color = if (e.incoming) T.accentFg else T.text2,
                    modifier = Modifier.clearAndSetSemantics { contentDescription = way },
                )
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
        if (page.total > page.entries.size) {
            Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
                Muted(stringResource(R.string.clip_history_showing, page.entries.size, page.total), Modifier.weight(1f))
                SoftButton(stringResource(R.string.clip_history_more), { limit += PAGE })
            }
        }
    }
}
