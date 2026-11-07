package com.mydomain.android.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.IntrinsicSize
import androidx.compose.ui.semantics.Role
import androidx.compose.foundation.clickable
import androidx.compose.ui.draw.clip
import android.content.Context
import android.text.format.Formatter
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.rememberScrollState
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Clear
import androidx.compose.material.icons.filled.Search
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Checkbox
import androidx.compose.material3.CheckboxDefaults
import androidx.compose.material3.FilterChip
import androidx.compose.material3.FilterChipDefaults
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.OutlinedTextFieldDefaults
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import com.mydomain.android.R
import com.mydomain.android.core.ActivityFeed
import com.mydomain.android.core.ActivityLog
import com.mydomain.android.core.ActivityPage
import com.mydomain.android.core.ActivityQuery
import com.mydomain.android.core.ActivityRow
import com.mydomain.android.core.ChatHistory
import com.mydomain.android.core.ClipLog
import com.mydomain.android.core.Core
import com.mydomain.android.core.DeviceKinds
import com.mydomain.android.core.Notes
import com.mydomain.android.core.ReadLater
import com.mydomain.android.core.Timers
import com.mydomain.android.core.TransferLog
import java.text.DateFormat
import java.util.Calendar
import java.util.Date

/**
 * Activity: one timeline of what happened between your devices, grouped by
 * day ([ActivityFeed] merges the histories the phone keeps with
 * [ActivityLog]'s own record). Words, a kind and a device narrow it; "Load
 * more" goes further back. Kinds can be hidden; "Clear activity" empties
 * only the timeline's own record (the histories stay). A row opens where it
 * lives ([onOpen]); the way to Transfers stays on top.
 */
@Composable
fun ActivityScreen(onTransfers: () -> Unit, onOpen: (ActivityRow) -> Unit) {
    val ctx = LocalContext.current
    var text by rememberSaveable { mutableStateOf("") }
    var kind by rememberSaveable { mutableStateOf("") }
    var node by rememberSaveable { mutableStateOf("") }
    var pages by rememberSaveable { mutableIntStateOf(1) }
    var choosing by remember { mutableStateOf(false) }
    var confirmClear by remember { mutableStateOf(false) }

    // Read here so the page is worked out again when any source changes.
    val sources = listOf(
        ActivityLog.version, ChatHistory.version, ClipLog.list, TransferLog.list,
        Notes.byId.size, ReadLater.byId.size, Timers.byId.size, Core.peers,
    )
    // The pages loaded so far, each going on from the last row of the one
    // before (worked out again together, so they never overlap).
    val page = remember(text, kind, node, pages, sources) {
        val q = ActivityQuery(node = node.ifEmpty { null }, kinds = if (kind.isEmpty()) emptySet() else setOf(kind), text = text)
        val rows = ArrayList<ActivityRow>()
        var more = true
        repeat(pages) {
            if (!more) return@repeat
            val last = rows.lastOrNull()
            val p = ActivityLog.page(q.copy(beforeTs = last?.ts, beforeId = last?.id))
            rows += p.items
            more = p.more
        }
        ActivityPage(rows, more)
    }
    val hidden = ActivityLog.hidden()
    val filtered = text.isNotBlank() || kind.isNotEmpty() || node.isNotEmpty()
    val days = remember(page.items) { ActivityFeed.byDay(page.items) { dayKey(it) } }

    LazyColumn(Modifier.fillMaxSize(), contentPadding = PaddingValues(16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
        item {
            PageHead(stringResource(R.string.activity_title), stringResource(R.string.activity_subtitle))
        }
        item {
            val active = TransferLog.list.count { !it.finished }
            SoftCard(padding = PaddingValues(4.dp)) {
                NavRow(
                    R.drawable.ic_lucide_arrow_left_right, Hues.files,
                    if (active > 0) stringResource(R.string.transfers_open_active, active) else stringResource(R.string.transfers_open),
                    stringResource(R.string.transfers_subtitle), onClick = onTransfers,
                )
            }
        }
        item {
            OutlinedTextField(
                text, { text = it; pages = 1 },
                modifier = Modifier.fillMaxWidth(),
                label = { Text(stringResource(R.string.activity_search)) },
                singleLine = true,
                leadingIcon = { Icon(Icons.Filled.Search, contentDescription = null, tint = T.text3) },
                trailingIcon = if (text.isEmpty()) null else {
                    { IconButton(onClick = { text = "" }) { Icon(Icons.Filled.Clear, stringResource(R.string.search_clear), tint = T.text2) } }
                },
                shape = MaterialTheme.shapes.small,
                colors = OutlinedTextFieldDefaults.colors(
                    focusedBorderColor = T.accent, unfocusedBorderColor = T.controlBorder,
                    focusedLabelColor = T.accentFg, unfocusedLabelColor = T.text3, cursorColor = T.accent,
                    focusedContainerColor = T.surface2, unfocusedContainerColor = T.surface2,
                    focusedTextColor = T.text, unfocusedTextColor = T.text,
                ),
            )
        }
        item {
            Chips(
                listOf("" to stringResource(R.string.activity_all)) +
                    ActivityFeed.KINDS.filter { it !in hidden }.map { it to stringResource(kindLabel(it)) },
                kind,
            ) { kind = it; pages = 1 }
        }
        if (Core.peers.isNotEmpty()) {
            item {
                Chips(
                    listOf("" to stringResource(R.string.activity_all_devices)) + Core.peers.map { it.nodeId to Core.peerName(it.nodeId) },
                    node,
                ) { node = it; pages = 1 }
            }
        }
        item {
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                GhostButton(stringResource(R.string.activity_choose), { choosing = true }, small = true)
                GhostButton(stringResource(R.string.activity_clear), { confirmClear = true }, small = true)
            }
        }
        ActivityLog.clearedAt()?.let { at ->
            item {
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Muted(
                        stringResource(R.string.activity_cleared_since, DateFormat.getDateTimeInstance(DateFormat.MEDIUM, DateFormat.SHORT).format(Date(at))),
                        Modifier.weight(1f),
                    )
                    TextButton(onClick = { ActivityLog.restore() }) { Text(stringResource(R.string.activity_show_earlier), color = T.accentFg) }
                }
            }
        }
        if (page.items.isEmpty()) {
            item {
                if (filtered) Muted(stringResource(R.string.activity_no_match), Modifier.padding(vertical = 8.dp).announce())
                else EmptyState(stringResource(R.string.activity_empty_title), stringResource(R.string.activity_empty_body))
            }
        }
        days.forEach { (day, rows) ->
            item(key = "d-$day") {
                Text(
                    dayLabel(ctx, rows.first().ts), Modifier.padding(top = 12.dp).semantics { heading() },
                    style = MaterialTheme.typography.titleSmall, color = T.text2,
                )
            }
            items(rows, key = { it.id }) { a -> ActivityRowView(ctx, a, onOpen, last = a === rows.last()) }
        }
        if (page.more) {
            item { GhostButton(stringResource(R.string.activity_load_more), { pages++ }, Modifier.fillMaxWidth()) }
        }
    }

    if (choosing) {
        AlertDialog(
            onDismissRequest = { choosing = false },
            containerColor = T.bar,
            title = { Text(stringResource(R.string.activity_choose_title), color = T.text) },
            text = {
                Column {
                    ActivityFeed.KINDS.forEach { k ->
                        val shown = k !in hidden
                        Row(verticalAlignment = Alignment.CenterVertically) {
                            Checkbox(
                                checked = shown,
                                onCheckedChange = { on ->
                                    ActivityLog.setHidden(if (on) hidden - k else hidden + k)
                                    if (!on && kind == k) kind = ""
                                },
                                colors = CheckboxDefaults.colors(checkedColor = T.accent),
                            )
                            Text(stringResource(kindLabel(k)), color = T.text)
                        }
                    }
                }
            },
            confirmButton = { TextButton(onClick = { choosing = false }) { Text(stringResource(R.string.activity_done), color = T.accentFg) } },
        )
    }
    if (confirmClear) {
        AlertDialog(
            onDismissRequest = { confirmClear = false },
            containerColor = T.bar,
            title = { Text(stringResource(R.string.activity_clear_title), color = T.text) },
            text = { Text(stringResource(R.string.activity_clear_body), color = T.text2) },
            dismissButton = { TextButton(onClick = { confirmClear = false }) { Text(stringResource(R.string.cancel), color = T.text2) } },
            confirmButton = {
                TextButton(onClick = { confirmClear = false; ActivityLog.clear(); pages = 1 }) {
                    Text(stringResource(R.string.activity_clear), color = T.danger)
                }
            },
        )
    }
}

/** Single-choice chips in a row that scrolls sideways. */
@Composable
private fun Chips(options: List<Pair<String, String>>, selected: String, onSelect: (String) -> Unit) {
    Pills(options.map { (value, label) -> Pill(value, label, if (value.isEmpty()) null else kindIcon(value)) }, selected, onSelect)
}

/** One row: a coloured round icon on the timeline, what happened, then the
 *  device, size, way and time. One TalkBack stop; tapping opens it where it
 *  lives. [last]: no line below its icon. */
@Composable
internal fun ActivityRowView(ctx: Context, a: ActivityRow, onOpen: (ActivityRow) -> Unit, last: Boolean = false) {
    val device = a.node?.let { Core.peerName(it) } ?: stringResource(R.string.activity_some_device)
    val meta = listOfNotNull(a.node?.let { Core.peerName(it) }) + activityMeta(ctx, a) +
        DateFormat.getTimeInstance(DateFormat.SHORT).format(Date(a.ts))
    val failed = a.action.endsWith("_failed") || a.action == "key_changed"
    val (icon, h) = activityIcon(a)
    val line = T.border
    Row(
        Modifier.fillMaxWidth().clip(MaterialTheme.shapes.small).clickable(role = Role.Button) { onOpen(a) }
            .height(IntrinsicSize.Min).padding(horizontal = 4.dp),
        verticalAlignment = Alignment.Top,
    ) {
        Box(Modifier.width(40.dp).fillMaxHeight(), contentAlignment = Alignment.TopCenter) {
            if (!last) Box(Modifier.padding(top = 40.dp).width(2.dp).fillMaxHeight().background(line))
            Box(Modifier.padding(top = 6.dp)) { IconBubble(icon, if (failed) Hue.Red else h, 34.dp) }
        }
        Spacer(Modifier.width(10.dp))
        Column(Modifier.weight(1f).padding(vertical = 8.dp)) {
            Text(
                activityText(ctx, a, device), style = MaterialTheme.typography.bodyMedium,
                color = if (failed) T.danger else T.text, maxLines = 2, overflow = TextOverflow.Ellipsis,
            )
            Row(verticalAlignment = Alignment.CenterVertically) {
                a.node?.let {
                    DeviceIcon(DeviceKinds.iconOf(it), 14.dp, T.text2)
                    Spacer(Modifier.width(4.dp))
                }
                Muted(meta.joinToString(" · "))
            }
        }
    }
}

/** A kind's icon (the filter chips). */
private fun kindIcon(kind: String): Int = when (kind) {
    "files" -> R.drawable.ic_lucide_file
    "clipboard" -> R.drawable.ic_lucide_clipboard
    "messages" -> R.drawable.ic_lucide_message_circle
    "calls" -> R.drawable.ic_lucide_phone
    "notifications" -> R.drawable.ic_lucide_bell
    "connections" -> R.drawable.ic_lucide_link_2
    else -> R.drawable.ic_lucide_activity
}

private fun kindLabel(kind: String): Int = when (kind) {
    "files" -> R.string.activity_kind_files
    "clipboard" -> R.string.activity_kind_clipboard
    "messages" -> R.string.activity_kind_messages
    "calls" -> R.string.activity_kind_calls
    "notifications" -> R.string.activity_kind_notifications
    "connections" -> R.string.activity_kind_connections
    else -> R.string.activity_kind_other
}

/** A row's icon and colour: files blue, messages violet, calls green,
 *  alerts amber, problems red. */
private fun activityIcon(a: ActivityRow): Pair<Int, Hue> = when {
    a.action == "note" -> R.drawable.ic_lucide_sticky_note to Hue.Amber
    a.action == "timer" -> R.drawable.ic_lucide_timer to Hue.Amber
    a.action == "read_later" -> R.drawable.ic_lucide_bookmark to Hue.Cyan
    a.action == "key_changed" -> R.drawable.ic_lucide_triangle_alert to Hue.Red
    a.kind == "files" -> R.drawable.ic_lucide_file to Hues.files
    a.kind == "clipboard" -> R.drawable.ic_lucide_clipboard to Hue.Violet
    a.kind == "messages" -> R.drawable.ic_lucide_message_circle to Hues.messages
    a.kind == "calls" -> R.drawable.ic_lucide_phone to Hues.calls
    a.kind == "notifications" -> R.drawable.ic_lucide_bell to Hues.alerts
    a.kind == "connections" -> R.drawable.ic_lucide_link_2 to Hue.Green
    else -> R.drawable.ic_lucide_activity to Hue.Muted
}

/** A timer's length: "5 min", "1 h 30 min", "45 s". */
private fun length(ctx: Context, ms: Long): String {
    val s = ms / 1000
    val h = s / 3600
    val m = (s % 3600) / 60
    val parts = ArrayList<String>()
    if (h > 0) parts += ctx.getString(R.string.activity_hours, h.toInt())
    if (m > 0) parts += ctx.getString(R.string.activity_minutes, m.toInt())
    if (h == 0L && m == 0L) parts += ctx.getString(R.string.activity_seconds, (s % 60).toInt())
    return parts.joinToString(" ")
}

/** What the row says, [device] being the other device's name. */
fun activityText(ctx: Context, a: ActivityRow, device: String): String {
    val s = a.subject
    val on = a.detail == "on"
    fun str(id: Int, vararg args: Any) = ctx.getString(id, *args)
    return when (a.action) {
        "file_sent" -> str(R.string.activity_row_file_sent, s, device)
        "file_received" -> str(R.string.activity_row_file_received, s, device)
        "file_send_failed" -> str(R.string.activity_row_file_send_failed, s, device)
        "file_receive_failed" -> str(R.string.activity_row_file_receive_failed, s, device)
        "file_sending" -> str(R.string.activity_row_file_sending, s, device)
        "file_receiving" -> str(R.string.activity_row_file_receiving, s, device)
        "link_downloaded" -> str(R.string.activity_row_link_downloaded, s, a.detail, device)
        "link_downloaded_there" -> str(R.string.activity_row_link_downloaded_there, device, s, a.detail)
        "clip_in" -> str(R.string.activity_row_clip_in, device, s)
        "clip_out" -> str(R.string.activity_row_clip_out, device, s)
        "msg_in" -> str(R.string.activity_row_msg_in, device, s)
        "msg_out" -> str(R.string.activity_row_msg_out, device, s)
        "msg_failed" -> str(R.string.activity_row_msg_failed, device, s)
        "call" -> {
            val caller = s.ifEmpty { str(R.string.activity_unknown_caller) }
            when (a.detail) {
                "ringing" -> str(R.string.activity_row_call_ringing, caller)
                "missed" -> str(R.string.activity_row_call_missed, caller)
                "ended" -> str(R.string.activity_row_call_ended, caller)
                else -> str(R.string.act_call, caller, a.detail)
            }
        }
        "link_up" -> str(R.string.act_connected, device)
        "link_down" -> str(R.string.act_disconnected, device)
        "key_changed" -> str(R.string.activity_row_key_changed, device)
        "wake" -> str(R.string.activity_row_wake, device)
        "note" -> str(R.string.activity_row_note, s.ifEmpty { str(R.string.activity_untitled) })
        "read_later" -> str(R.string.activity_row_read_later, s)
        "timer" -> {
            val len = length(ctx, a.detail.toLongOrNull() ?: 0L)
            if (s.isNotEmpty()) str(R.string.activity_row_timer, s, len) else str(R.string.activity_row_timer_no_label, len)
        }
        "ring" -> str(R.string.act_ring, device)
        "open_url" -> str(R.string.act_link, device, s)
        "screen_watch" -> str(if (on) R.string.activity_row_screen_watch_on else R.string.activity_row_screen_watch_off, device)
        "screen_offer" -> str(if (on) R.string.activity_row_screen_offer_on else R.string.activity_row_screen_offer_off, device)
        "camera_watch" -> str(if (on) R.string.activity_row_camera_watch_on else R.string.activity_row_camera_watch_off, device)
        "audio_offer" -> str(if (on) R.string.activity_row_audio_offer_on else R.string.activity_row_audio_offer_off, device)
        "audio_listen" -> str(if (on) R.string.activity_row_audio_listen_on else R.string.activity_row_audio_listen_off, device)
        else -> s.ifEmpty { a.action }
    }
}

/** The bits after the sentence: size, how it went, the site. */
private fun activityMeta(ctx: Context, a: ActivityRow): List<String> = buildList {
    a.bytes?.let { add(Formatter.formatShortFileSize(ctx, it)) }
    when (a.via) {
        "wifi_direct" -> add(ctx.getString(R.string.transfers_via_wifi_direct))
        "hotspot" -> add(ctx.getString(R.string.transfers_via_hotspot))
    }
    if (a.action == "link_up" && a.detail == "relay") add(ctx.getString(R.string.activity_via_relay))
    if (a.action == "link_up" && a.detail == "ble") add(ctx.getString(R.string.activity_via_bluetooth))
    if (a.action == "read_later" && a.detail.isNotEmpty()) add(a.detail)
}

/** A row's day, as a key: days since 1970 in local time. */
private fun dayKey(ts: Long): Long {
    val c = Calendar.getInstance().apply { timeInMillis = ts }
    return c.get(Calendar.YEAR) * 1000L + c.get(Calendar.DAY_OF_YEAR)
}

/** Today, Yesterday, or the date. */
private fun dayLabel(ctx: Context, ts: Long): String {
    val today = dayKey(System.currentTimeMillis())
    val yesterday = dayKey(System.currentTimeMillis() - 24 * 3600 * 1000L)
    return when (dayKey(ts)) {
        today -> ctx.getString(R.string.activity_today)
        yesterday -> ctx.getString(R.string.activity_yesterday)
        else -> DateFormat.getDateInstance(DateFormat.FULL).format(Date(ts))
    }
}

/** Home's Recent activity: the newest [n] rows, each opening where it lives. */
@Composable
fun RecentActivity(n: Int, onOpen: (ActivityRow) -> Unit) {
    val ctx = LocalContext.current
    val sources = listOf(
        ActivityLog.version, ChatHistory.version, ClipLog.list, TransferLog.list,
        Notes.byId.size, ReadLater.byId.size, Timers.byId.size, Core.peers,
    )
    val rows = remember(sources) { ActivityLog.page(ActivityQuery(limit = n)).items.take(n) }
    if (rows.isEmpty()) {
        Muted(stringResource(R.string.activity_empty_body), Modifier.padding(8.dp))
        return
    }
    Column {
        rows.forEach { a -> ActivityRowView(ctx, a, onOpen, last = a === rows.last()) }
    }
}
