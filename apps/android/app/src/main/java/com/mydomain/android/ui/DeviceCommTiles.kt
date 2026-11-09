package com.mydomain.android.ui

import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.heightIn
import androidx.compose.material3.IconButtonDefaults
import androidx.compose.material3.FilledIconButton
import androidx.compose.material3.TextButton
import java.util.Locale
import androidx.compose.foundation.layout.height
import androidx.compose.runtime.derivedStateOf
import android.content.Context
import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.activity.compose.BackHandler
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.DropdownMenu
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.ui.semantics.selected
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.onLongClick
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.lazy.LazyListScope
import com.mydomain.android.core.DeviceIconShapes
import com.mydomain.android.core.DeviceKinds
import com.mydomain.android.core.DeviceLabels
import com.mydomain.android.core.LayoutScope
import com.mydomain.android.core.PageArrangement
import com.mydomain.android.core.PageLayout
import com.mydomain.android.core.PageLayoutStore
import com.mydomain.android.core.PageLayoutView
import androidx.compose.ui.platform.LocalContext
import android.net.Uri
import android.content.Intent
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
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import org.json.JSONObject
import com.mydomain.android.core.DeviceStatus
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import com.mydomain.android.ComputerNotifs
import com.mydomain.android.PhoneAudio
import com.mydomain.android.R
import com.mydomain.android.core.ChatHistory
import com.mydomain.android.core.ChatMessage
import com.mydomain.android.core.ChatPage
import com.mydomain.android.core.ChatStatus
import com.mydomain.android.core.ClipLog
import com.mydomain.android.core.Core
import com.mydomain.android.core.LinkState
import com.mydomain.android.core.friendlyError
import com.mydomain.android.core.linkStateOf
import com.mydomain.android.core.progressLabel
import kotlinx.coroutines.launch

// The device page's Messages and Files tiles (DeviceTiles.kt lists them).

/** Saved messages a chat shows at first, and adds per step back. */
private const val CHAT_PAGE = 100

/** The chat with [node]; [found]: a message a search opened (marked). */
@Composable
internal fun ChatTile(node: String, found: Long? = null) {
    val toast = LocalToast.current
    val scope = rememberCoroutineScope()
    var text by rememberSaveable(node) { mutableStateOf("") }
    var busy by remember { mutableStateOf(false) }
    // The newest [CHAT_PAGE] saved messages ([ChatHistory]), [CHAT_PAGE]
    // more per "Show older messages".
    var shown by remember(node) { mutableIntStateOf(CHAT_PAGE) }
    val keep = ChatHistory.enabled
    val saved = if (keep) ChatHistory.page(node, null, shown) else ChatPage(emptyList(), false, 0)
    val live = Core.messages.filter { it.node == node }
    // This session's bubbles carry the live state; a saved copy of one is left out.
    val liveIds = live.mapNotNull { it.msgId }.toHashSet()
    val older = saved.entries.filter { it.id !in liveIds }.map {
        ChatMessage(
            it.bubble, it.node, it.incoming, it.text, it.at, it.id,
            delivered = it.status == ChatStatus.Delivered,
            failed = it.status == ChatStatus.Failed,
            fromHistory = true,
        )
    }
    // "Clear" waiting on its Undo: what it will remove is hidden already.
    val clearKey = "chat:$node"
    val merged = (older + live).filter { !PendingClears.hides(clearKey, it.ts) }.sortedBy { it.ts }
    val thread = merged.takeLast(if (keep) shown else 50)
    val undo = LocalUndo.current
    val cleared = stringResource(R.string.chat_cleared)
    val undoLabel = stringResource(R.string.notes_undo)
    val more = keep && !PendingClears.pending(clearKey) && (saved.more || merged.size > shown)

    Tile {
        // The title, with "Clear" kept small at its end, away from the input.
        Row(verticalAlignment = Alignment.CenterVertically) {
            CardTitle(stringResource(R.string.chat_title), Modifier.weight(1f))
            if (thread.isNotEmpty()) {
                TextButton({ PendingClears.ask(undo, clearKey, cleared, undoLabel) { Core.clearChat(node) } }, Modifier.heightIn(min = 48.dp)) {
                    Text(stringResource(R.string.chat_clear), color = T.text2, style = MaterialTheme.typography.labelLarge)
                }
            }
        }
        if (thread.isEmpty()) {
            Muted(stringResource(R.string.chat_empty), Modifier.fillMaxWidth().padding(vertical = 8.dp), center = true)
        }
        if (more) {
            Box(Modifier.fillMaxWidth(), contentAlignment = Alignment.Center) {
                GhostButton(stringResource(R.string.chat_older), { shown += CHAT_PAGE }, small = true)
            }
        }
        thread.forEach { m ->
            val waiting = m.queued && m.pending
            val way = stringResource(if (m.incoming) R.string.transfers_dir_received else R.string.transfers_dir_sent)
            Box(Modifier.fillMaxWidth(), contentAlignment = if (m.incoming) Alignment.CenterStart else Alignment.CenterEnd) {
                Column(
                    // One stop per message, saying which way it went.
                    Modifier.semantics(mergeDescendants = true) { stateDescription = way }
                        .foundMark(m.id == found)
                        .widthIn(max = 280.dp)
                        .alpha(if (waiting) 0.7f else 1f)
                        .background(if (m.incoming) T.surface2 else T.primary, MaterialTheme.shapes.medium)
                        .padding(horizontal = 12.dp, vertical = 8.dp),
                ) {
                    Text(m.text, color = if (m.incoming) T.text else T.onPrimary, style = MaterialTheme.typography.bodyMedium)
                    if (waiting) {
                        Text(stringResource(R.string.chat_sends_when_connected), color = T.onPrimary, style = MaterialTheme.typography.labelMedium)
                    }
                    if (m.failed) {
                        Text(stringResource(R.string.chat_failed), color = T.onPrimary, style = MaterialTheme.typography.labelMedium)
                    }
                }
            }
        }
        if (keep) Muted(stringResource(R.string.chat_kept))
        Row(verticalAlignment = Alignment.CenterVertically) {
            Field(text, { text = it }, stringResource(R.string.chat_hint), Modifier.weight(1f))
            Spacer(Modifier.width(8.dp))
            // Send: a round icon button (one primary action per tab).
            val canSend = !busy && text.isNotBlank()
            FilledIconButton(
                onClick = {
                    val t = text.trim()
                    if (t.isNotEmpty()) {
                        busy = true
                        scope.launch {
                            try { Core.sendMessage(node, t); text = "" } catch (e: Exception) { toast(friendlyError(e)) } finally { busy = false }
                        }
                    }
                },
                enabled = canSend,
                modifier = Modifier.size(48.dp),
                colors = IconButtonDefaults.filledIconButtonColors(
                    containerColor = T.primary, contentColor = T.onPrimary,
                    disabledContainerColor = T.surface2, disabledContentColor = T.text3,
                ),
            ) { LIcon(R.drawable.ic_lucide_send, if (canSend) T.onPrimary else T.text3, 20.dp, stringResource(R.string.send)) }
        }
    }
}

@Composable
internal fun ClipboardTile(node: String) {
    val toast = LocalToast.current
    val scope = rememberCoroutineScope()
    var busy by remember { mutableStateOf(false) }
    val asked = stringResource(R.string.clipboard_asked)
    Tile(title = stringResource(R.string.clipboard_title)) {
        Muted(stringResource(R.string.clipboard_body))
        SoftButton(stringResource(if (busy) R.string.clipboard_getting else R.string.clipboard_get), {
            busy = true
            scope.launch {
                try { Core.pullClipboard(node); toast(asked) } catch (e: Exception) { toast(friendlyError(e)) } finally { busy = false }
            }
        }, Modifier.fillMaxWidth(), enabled = !busy)
        val ctx = androidx.compose.ui.platform.LocalContext.current
        val pictureSent = stringResource(R.string.clipboard_picture_sent)
        val noPicture = stringResource(R.string.clipboard_no_picture)
        SoftButton(stringResource(R.string.clipboard_send_picture), {
            scope.launch {
                try {
                    if (Core.sendClipboardImage(ctx, node)) toast(pictureSent) else toast(noPicture)
                } catch (e: Exception) { toast(friendlyError(e)) }
            }
        }, Modifier.fillMaxWidth())
        ClipSyncSwitch(stringResource(R.string.clipboard_sync))
        ClipHistory()
    }
}

/** [PendingClears] key of the recent clipboard values. */
private const val RECENT_CLIPS = "recent-clips"

/** Recent clipboard values (memory only), each with a Copy button. */
@Composable
private fun ClipHistory() {
    val toast = LocalToast.current
    val copied = stringResource(R.string.clipboard_copied)
    val undo = LocalUndo.current
    val cleared = stringResource(R.string.clipboard_recent_cleared)
    val undoLabel = stringResource(R.string.notes_undo)
    val recent = Core.clipHistory.filter { !PendingClears.hides(RECENT_CLIPS, it.ts) }.take(8)
    if (recent.isEmpty()) return
    Row(verticalAlignment = Alignment.CenterVertically) {
        Text(
            stringResource(R.string.clipboard_recent), style = MaterialTheme.typography.labelLarge, color = T.text,
            modifier = Modifier.weight(1f).semantics { heading() },
        )
        GhostButton(stringResource(R.string.clipboard_clear), {
            PendingClears.ask(undo, RECENT_CLIPS, cleared, undoLabel) { Core.clearClipHistory() }
        }, small = true)
    }
    recent.forEach { e ->
        Row(verticalAlignment = Alignment.CenterVertically) {
            Column(Modifier.weight(1f).semantics(mergeDescendants = true) {}) {
                Text(e.text, style = MaterialTheme.typography.bodyMedium, color = T.text, maxLines = 1, overflow = TextOverflow.Ellipsis)
                Text(
                    if (e.source == null) stringResource(R.string.clipboard_from_this_phone) else Core.peerName(e.source),
                    style = MaterialTheme.typography.labelMedium, color = T.text2, maxLines = 1, overflow = TextOverflow.Ellipsis,
                )
            }
            Spacer(Modifier.width(8.dp))
            GhostButton(stringResource(R.string.clipboard_copy), {
                runCatching { Core.copyFromHistory(e) }.onSuccess { toast(copied) }.onFailure { toast(friendlyError(it)) }
            }, small = true)
        }
    }
    Muted(stringResource(R.string.clipboard_recent_hint))
}

/** A computer's own notifications on this phone ([ComputerNotifs]): on
 *  unless turned off here; [sharing] is the computer's own switch. */
@Composable
internal fun ComputerNotifsTile(node: String, name: String, sharing: Boolean) {
    val ctx = LocalContext.current
    var on by remember(node) { mutableStateOf(ComputerNotifs.isOn(ctx, node)) }
    Tile(title = stringResource(R.string.computer_notifs_title)) {
        Muted(stringResource(R.string.computer_notifs_body, name))
        SwitchRow(stringResource(R.string.computer_notifs_switch), on) {
            on = it
            ComputerNotifs.setOn(ctx, node, it)
        }
        if (!sharing) Muted(stringResource(R.string.computer_notifs_off_there, name))
    }
}

@Composable
internal fun NotifyTile() {
    val toast = LocalToast.current
    val scope = rememberCoroutineScope()
    var title by remember { mutableStateOf("") }
    var body by remember { mutableStateOf("") }
    val sent = stringResource(R.string.notify_sent)
    Tile(title = stringResource(R.string.notify_title)) {
        Field(title, { title = it }, stringResource(R.string.notify_field_title))
        Field(body, { body = it }, stringResource(R.string.notify_field_message))
        SoftButton(stringResource(R.string.notify_send_all), {
            val t = title.trim()
            scope.launch {
                try { Core.shareNotification(t, body.trim()); title = ""; body = ""; toast(sent) } catch (e: Exception) { toast(friendlyError(e)) }
            }
        }, Modifier.fillMaxWidth(), enabled = title.isNotBlank())
    }
}

@Composable
internal fun AppsTile(node: String) {
    val toast = LocalToast.current
    val scope = rememberCoroutineScope()
    var q by remember { mutableStateOf("") }
    val data = Core.apps[node]
    Tile(title = stringResource(R.string.apps_title)) {
        Muted(stringResource(R.string.apps_body))
        if (data == null) {
            SoftButton(stringResource(R.string.apps_show), {
                scope.launch { try { Core.requestApps(node) } catch (e: Exception) { toast(friendlyError(e)) } }
            })
        } else {
            Field(q, { q = it }, stringResource(R.string.apps_search))
            val shown = data.apps.filter { it.label.contains(q, ignoreCase = true) }.take(50)
            if (shown.isEmpty()) Muted(stringResource(R.string.apps_none))
            shown.forEach { a ->
                SwitchRow(a.label, a.pkg in data.subscribed) {
                    scope.launch { try { Core.toggleApp(node, a.pkg) } catch (e: Exception) { toast(friendlyError(e)) } }
                }
            }
        }
    }
}
