package com.mydomain.android.ui

import androidx.compose.ui.semantics.selected
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.onLongClick
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.lazy.LazyListScope
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
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material3.Icon
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

/** One device: its status, then a tile per feature. [focus]: what a
 *  search result opened ("msg:<id>" a message, "clip:<id>" a clip), brought
 *  into view and marked. */
@Composable
fun DeviceScreen(node: String, onBack: () -> Unit, focus: String? = null) {
    val peer = Core.peers.firstOrNull { it.nodeId == node }
    val toast = LocalToast.current
    val scope = rememberCoroutineScope()
    var editing by remember(node) { mutableStateOf(false) }
    val list = rememberLazyListState()
    val foundMessage = focus?.takeIf { it.startsWith("msg:") }?.removePrefix("msg:")?.toLongOrNull()
    val foundClip = focus?.takeIf { it.startsWith("clip:") }?.removePrefix("clip:")?.toLongOrNull()
    // Scroll to the tile; the row in it brings itself into view (foundMark).
    LaunchedEffect(node, focus) {
        when {
            foundMessage != null -> list.scrollToKey("chat")
            foundClip != null -> list.scrollToKey("clipboard-history")
        }
    }
    // Arrange (a long press on a tile, or the button by its name): the
    // screen's tiles in the saved order, for this kind of device or this
    // device alone (PageLayoutStore).
    var arranging by remember(node) { mutableStateOf(false) }
    var said by remember(node) { mutableStateOf("") }
    val kind = pageKind(node)
    val tiles = deviceTiles(node, Core.peerName(node), foundMessage, foundClip).filter { kind in it.kinds }
    val layout = PageLayoutStore.view(kind, node, tiles.map { it.id })
    val arrangeLabel = stringResource(R.string.layout_arrange)

    Column(Modifier.fillMaxSize().imePadding()) {
        Row(Modifier.fillMaxWidth().padding(start = 4.dp, top = 4.dp), verticalAlignment = Alignment.CenterVertically) {
            IconButton(onClick = onBack) { Icon(Icons.AutoMirrored.Filled.ArrowBack, stringResource(R.string.back), tint = T.text) }
            Text(stringResource(R.string.home_title), style = MaterialTheme.typography.bodyMedium, color = T.text2)
        }
        if (peer == null) {
            EmptyState(stringResource(R.string.device_not_found_title), stringResource(R.string.device_not_found_body))
            return
        }
        val s = linkStateOf(node)
        // The nickname you gave it here, else its own name.
        val name = Core.peerName(node)
        LazyColumn(
            Modifier.fillMaxSize(),
            state = list,
            contentPadding = PaddingValues(start = 16.dp, end = 16.dp, bottom = 16.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            item {
                Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.padding(vertical = 4.dp)) {
                    DeviceAvatar(node, 56.dp)
                    Spacer(Modifier.width(14.dp))
                    Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(3.dp)) {
                        Row(verticalAlignment = Alignment.CenterVertically) {
                            Text(
                                name, style = MaterialTheme.typography.headlineSmall, color = T.text,
                                maxLines = 1, overflow = TextOverflow.Ellipsis,
                                modifier = Modifier.weight(1f, fill = false).semantics { heading() },
                            )
                            Spacer(Modifier.width(8.dp))
                            GhostButton(stringResource(R.string.label_edit), { editing = !editing }, small = true)
                            Spacer(Modifier.width(6.dp))
                            GhostButton(
                                stringResource(if (arranging) R.string.layout_done else R.string.layout_arrange),
                                { arranging = !arranging; said = "" },
                                small = true,
                            )
                        }
                        StatusPill(node)
                        if (s == LinkState.Connecting || s == LinkState.Failed) {
                            progressLabel(Core.progress[node])?.let { Muted(it, Modifier.announce()) }
                        }
                        // Asleep? Wake it (Wake-on-LAN), if it said it may be woken.
                        WakeIt(node)
                    }
                    if (s == LinkState.Offline || s == LinkState.Failed) {
                        PrimaryButton(stringResource(R.string.connect), {
                            scope.launch { try { Core.connect(node) } catch (e: Exception) { toast(friendlyError(e)) } }
                        })
                    }
                    // Connected: drop it, or drop it and connect afresh.
                    if (s == LinkState.Direct || s == LinkState.Relay) {
                        Column(horizontalAlignment = Alignment.End) {
                            GhostButton(stringResource(R.string.reconnect), {
                                scope.launch { try { Core.reconnect(node) } catch (e: Exception) { toast(friendlyError(e)) } }
                            }, small = true)
                            GhostButton(stringResource(R.string.disconnect), {
                                scope.launch { try { Core.disconnect(node) } catch (e: Exception) { toast(friendlyError(e)) } }
                            }, small = true)
                        }
                    }
                }
            }
            // Its security key changed: refused until you decide.
            item(key = "key-change") { KeyChangeBanner(node) }
            if (editing) item { DeviceLabelEditor(node) { editing = false } }
            if (arranging) {
                arrangeItems(node, name, kind, tiles, layout, said, { said = it }) { arranging = false }
            } else {
                val shown = layout.shown.mapNotNull { id -> tiles.firstOrNull { it.id == id } }.filter { it.available }
                for (t in shown) item(key = t.id) {
                    Box(Modifier.longPressToArrange(node, arrangeLabel) { arranging = true }) { t.content() }
                }
                if (layout.shown.isEmpty()) item { Muted(stringResource(R.string.layout_all_hidden)) }
            }
        }
    }
}

/** "phone" or "computer": which layout [node]'s screen uses. Its own report
 *  first, then the icon you gave it here; a computer when nothing says. */
private fun pageKind(node: String): String {
    Core.deviceStatus[node]?.platform?.let { return if (it == "android") "phone" else "computer" }
    val icon = DeviceLabels.labels[node]?.icon
    return if (icon == "phone" || icon == "tablet" || icon == "watch") "phone" else "computer"
}

private val BOTH = setOf("phone", "computer")
private val COMPUTER = setOf("computer")

/** One tile a device's screen can show: [id] is kept in saved layouts
 *  (never rename one); [available]: the device offers it now (else it's
 *  left out, and listed as such in Arrange). */
private class PageTile(
    val id: String,
    val title: Int,
    val kinds: Set<String> = BOTH,
    val available: Boolean = true,
    val content: @Composable () -> Unit,
)

/** Every tile a device's screen can show, in default order. A new tile is
 *  one entry here; a saved layout that predates it shows it at this place. */
private fun deviceTiles(node: String, name: String, foundMessage: Long?, foundClip: Long?): List<PageTile> {
    val st = Core.deviceStatus[node]
    return listOf(
        PageTile("chat", R.string.chat_title) { ChatTile(node, foundMessage) },
        PageTile("files", R.string.files_title) { FilesTile(node) },
        // Its shared folders, for a device that can be browsed.
        PageTile("browse", R.string.browse_title, available = st?.sharedFolders != null) { BrowseTile(node, name) },
        PageTile("control", R.string.find_title) { FindTile(node, name) },
        // What it said about itself: battery, storage, memory, processor, uptime.
        PageTile("info", R.string.info_title, available = st != null && hasInfo(st)) {
            st?.let { InfoTile(node, name, it) }
        },
        // How this phone reaches the computer, and how fast.
        PageTile("connection", R.string.conn_title, COMPUTER) { ConnectionTile(node, name) },
        // Commands its owner lets your devices run (it lists "run").
        PageTile("commands", R.string.run_title, available = st?.actions?.contains("run") == true) { CommandsTile(node, name) },
        // A computer that lets the phone control it (it lists "input").
        PageTile("touchpad", R.string.touchpad_title, COMPUTER, available = st?.actions?.contains("input") == true) {
            st?.let { TouchpadTile(node, it) }
        },
        // A computer that can show this phone's screen (it says screen_view).
        PageTile("screen-share", R.string.screen_share_title, COMPUTER, available = st?.screenView == true) {
            ShareScreenTile(node, name)
        },
        // A computer that plays this phone's audio (it says audio_sink),
        // from a phone that can capture it (Android 10 or later).
        // (Phones say audio_sink too now, for a computer's sound: only
        // a computer's page offers this.)
        PageTile(
            "phone-audio", R.string.audio_tile_title, COMPUTER,
            available = st != null && st.audioSink && st.platform != "android" && PhoneAudio.supported,
        ) { PhoneAudioTile(node, name) },
        // A computer that plays its sound here (it says audio_source).
        PageTile(
            "computer-audio", R.string.caudio_tile_title, COMPUTER,
            available = st != null && st.audioSource && st.platform != "android",
        ) { ComputerAudioTile(node, name) },
        // A computer that can show its own notifications here (it says notif_share).
        PageTile("computer-notifs", R.string.computer_notifs_title, COMPUTER, available = st?.notifShare != null) {
            st?.notifShare?.let { ComputerNotifsTile(node, name, it) }
        },
        PageTile("clipboard", R.string.clipboard_title) { ClipboardTile(node) },
        // Text sent to it and from it as clipboard, kept on this phone.
        PageTile("clipboard-history", R.string.clip_history_title, available = ClipLog.enabled) {
            ClipHistoryTile(node, name, foundClip)
        },
        PageTile("notify", R.string.notify_title) { NotifyTile() },
        PageTile("apps", R.string.apps_title) { AppsTile(node) },
    )
}

/** A long press on a tile (not on a button in it) starts Arrange; TalkBack
 *  offers it as the tile's long-press action. */
private fun Modifier.longPressToArrange(node: String, label: String, onArrange: () -> Unit): Modifier = this
    .pointerInput(node) { detectTapGestures(onLongPress = { onArrange() }) }
    .semantics { onLongClick(label) { onArrange(); true } }

/** Arrange: each shown tile as a card with Move up / Move down / Hide, the
 *  hidden ones listed after to show again, "Only for this device" and
 *  Reset. Saved as it changes. */
private fun LazyListScope.arrangeItems(
    node: String,
    name: String,
    kind: String,
    tiles: List<PageTile>,
    layout: PageLayoutView,
    said: String,
    say: (String) -> Unit,
    onDone: () -> Unit,
) {
    val onlyThis = layout.scope == LayoutScope.DEVICE
    val byId = tiles.associateBy { it.id }
    val shown = layout.shown
    item(key = "arrange-bar") {
        val ctx = LocalContext.current
        Tile(title = stringResource(R.string.layout_title)) {
            Muted(stringResource(R.string.layout_hint))
            SwitchRow(stringResource(R.string.layout_only_this), onlyThis) { on ->
                // On: this device keeps what it shows now as its own. Off: it
                // follows its kind's layout again.
                PageLayoutStore.set(ctx, kind, node, true, if (on) PageLayout(layout.order, layout.hidden) else null)
            }
            Muted(
                when {
                    onlyThis -> stringResource(R.string.layout_only_this_on, name)
                    kind == "phone" -> stringResource(R.string.layout_all_phones)
                    else -> stringResource(R.string.layout_all_computers)
                },
            )
            val resetDone = stringResource(R.string.layout_reset_done)
            Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                if (layout.scope != LayoutScope.DEFAULT) {
                    GhostButton(stringResource(R.string.layout_reset), {
                        PageLayoutStore.set(ctx, kind, node, onlyThis, null)
                        say(resetDone)
                    })
                }
                Spacer(Modifier.weight(1f))
                PrimaryButton(stringResource(R.string.layout_done), onDone)
            }
            if (said.isNotEmpty()) Muted(said, Modifier.announce())
        }
    }
    for (id in shown) item(key = "arrange:$id") {
        val t = byId[id] ?: return@item
        val ctx = LocalContext.current
        val title = stringResource(t.title)
        val at = shown.indexOf(id)
        val upLabel = stringResource(R.string.layout_up_label, title)
        val downLabel = stringResource(R.string.layout_down_label, title)
        val hideLabel = stringResource(R.string.layout_hide_label, title)
        val hiddenNow = stringResource(R.string.layout_hidden_now, title)
        val res = ctx.resources
        fun step(s: Int) {
            val order = PageArrangement.moveStep(layout.order, layout.hidden, id, s)
            if (order === layout.order) return
            PageLayoutStore.set(ctx, kind, node, onlyThis, PageLayout(order, layout.hidden))
            val now = order.filter { it !in layout.hidden }
            say(res.getString(R.string.layout_moved_to, title, now.indexOf(id) + 1, now.size))
        }
        Tile(title = title) {
            if (!t.available) Muted(stringResource(R.string.layout_not_now, name))
            Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                SoftButton(
                    stringResource(R.string.layout_up), { step(-1) },
                    Modifier.semantics { contentDescription = upLabel }, enabled = at > 0,
                )
                SoftButton(
                    stringResource(R.string.layout_down), { step(1) },
                    Modifier.semantics { contentDescription = downLabel }, enabled = at < shown.size - 1,
                )
                Spacer(Modifier.weight(1f))
                GhostButton(stringResource(R.string.layout_hide), {
                    PageLayoutStore.set(ctx, kind, node, onlyThis, PageLayout(layout.order, layout.hidden + id))
                    say(hiddenNow)
                }, Modifier.semantics { contentDescription = hideLabel }, small = true)
            }
        }
    }
    val hidden = layout.order.filter { it in layout.hidden }
    if (hidden.isNotEmpty()) item(key = "arrange-hidden") {
        val ctx = LocalContext.current
        Tile(title = stringResource(R.string.layout_hidden)) {
            for (id in hidden) {
                val t = byId[id] ?: continue
                val title = stringResource(t.title)
                val showLabel = stringResource(R.string.layout_show_label, title)
                val shownNow = stringResource(R.string.layout_shown_now, title)
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Text(title, Modifier.weight(1f), style = MaterialTheme.typography.bodyMedium, color = T.text)
                    GhostButton(stringResource(R.string.layout_show), {
                        PageLayoutStore.set(ctx, kind, node, onlyThis, PageLayout(layout.order, layout.hidden - id))
                        say(shownNow)
                    }, Modifier.semantics { contentDescription = showLabel }, small = true)
                }
            }
        }
    }
}

/** Saved messages a chat shows at first, and adds per step back. */
private const val CHAT_PAGE = 100

/** The chat with [node]; [found]: a message a search opened (marked). */
@Composable
private fun ChatTile(node: String, found: Long? = null) {
    val toast = LocalToast.current
    val scope = rememberCoroutineScope()
    var text by remember { mutableStateOf("") }
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
    val merged = (older + live).sortedBy { it.ts }
    val thread = merged.takeLast(if (keep) shown else 50)
    val more = keep && (saved.more || merged.size > shown)

    Tile(title = stringResource(R.string.chat_title)) {
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
        if (thread.isNotEmpty()) {
            Box(Modifier.fillMaxWidth(), contentAlignment = Alignment.CenterEnd) {
                GhostButton(stringResource(R.string.chat_clear), { Core.clearChat(node) }, small = true)
            }
        }
        if (keep) Muted(stringResource(R.string.chat_kept))
        Row(verticalAlignment = Alignment.CenterVertically) {
            Field(text, { text = it }, stringResource(R.string.chat_hint), Modifier.weight(1f))
            Spacer(Modifier.width(8.dp))
            PrimaryButton(stringResource(R.string.send), {
                val t = text.trim()
                if (t.isNotEmpty()) {
                    busy = true
                    scope.launch {
                        try { Core.sendMessage(node, t); text = "" } catch (e: Exception) { toast(friendlyError(e)) } finally { busy = false }
                    }
                }
            }, enabled = !busy && text.isNotBlank())
        }
    }
}

@Composable
private fun ClipboardTile(node: String) {
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

/** Recent clipboard values (memory only), each with a Copy button. */
@Composable
private fun ClipHistory() {
    val toast = LocalToast.current
    val copied = stringResource(R.string.clipboard_copied)
    val recent = Core.clipHistory.take(8)
    if (recent.isEmpty()) return
    Row(verticalAlignment = Alignment.CenterVertically) {
        Text(
            stringResource(R.string.clipboard_recent), style = MaterialTheme.typography.labelLarge, color = T.text,
            modifier = Modifier.weight(1f).semantics { heading() },
        )
        GhostButton(stringResource(R.string.clipboard_clear), { Core.clearClipHistory() }, small = true)
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
private fun ComputerNotifsTile(node: String, name: String, sharing: Boolean) {
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
private fun NotifyTile() {
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
private fun AppsTile(node: String) {
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

/** Where another device is (find my computer): Locate, the position, and
 *  a map. Greyed out, saying why, until that device allows it. */
@Composable
private fun LocateRow(node: String, name: String, status: DeviceStatus) {
    val toast = LocalToast.current
    val scope = rememberCoroutineScope()
    val ctx = LocalContext.current
    val asking = stringResource(R.string.locate_asking, name)
    val place = status.location
    if (place != null) {
        val mins = ((System.currentTimeMillis() / 1000 - place.at) / 60).coerceAtLeast(0)
        Text(
            stringResource(R.string.locate_where, "%.5f".format(place.lat), "%.5f".format(place.lon), place.acc.toInt(), mins),
            Modifier.announce(), style = MaterialTheme.typography.bodyMedium, color = T.text,
        )
    }
    Row(verticalAlignment = Alignment.CenterVertically) {
        SoftButton(stringResource(R.string.locate), {
            scope.launch {
                try {
                    Core.runAction(node, "locate")
                    toast(asking)
                } catch (e: Exception) { toast(friendlyError(e)) }
            }
        }, Modifier.weight(1f), enabled = status.locationSharing)
        if (place != null) {
            Spacer(Modifier.width(8.dp))
            GhostButton(stringResource(R.string.locate_map), {
                val geo = Intent(Intent.ACTION_VIEW, Uri.parse("geo:${place.lat},${place.lon}?q=${place.lat},${place.lon}"))
                val web = Intent(Intent.ACTION_VIEW, Uri.parse("https://www.openstreetmap.org/?mlat=${place.lat}&mlon=${place.lon}#map=17/${place.lat}/${place.lon}"))
                runCatching { ctx.startActivity(geo) }.onFailure { runCatching { ctx.startActivity(web) } }
            })
        }
    }
    if (!status.locationSharing) Muted(stringResource(R.string.locate_off, name))
}

@Composable
private fun FindTile(node: String, name: String) {
    val toast = LocalToast.current
    val scope = rememberCoroutineScope()
    var url by remember { mutableStateOf("") }
    var busy by remember { mutableStateOf(false) }
    val status = Core.deviceStatus[node]
    val ringing = stringResource(R.string.find_ringing, name)
    val ringQueued = stringResource(R.string.find_ring_queued, name)
    val linkSent = stringResource(R.string.find_link_sent, name)
    val linkQueued = stringResource(R.string.find_link_queued, name)
    val linkInvalid = stringResource(R.string.find_link_invalid)

    // Ask for its status when the screen opens; the reply fills `status`
    // (and the info tile).
    // Asked only over a live link (and again when one comes up): asking a
    // device that isn't connected would wake it by push just to show this.
    val linked = Core.links.containsKey(node)
    LaunchedEffect(node, linked) { if (linked) runCatching { Core.requestStatus(node) } }

    Tile(title = stringResource(R.string.find_title)) {
        Muted(stringResource(R.string.find_body))
        if (status != null && status.actions.isNotEmpty()) Controls(node, status)
        SoftButton(stringResource(R.string.find_ring), {
            scope.launch {
                try {
                    val via = Core.ring(node)
                    toast(if (via == "queued") ringQueued else ringing)
                } catch (e: Exception) { toast(friendlyError(e)) }
            }
        }, Modifier.fillMaxWidth())
        if (status != null && "locate" in status.actions) LocateRow(node, name, status)
        Row(verticalAlignment = Alignment.CenterVertically) {
            Field(url, { url = it }, stringResource(R.string.find_link_hint), Modifier.weight(1f))
            Spacer(Modifier.width(8.dp))
            PrimaryButton(stringResource(R.string.send), {
                val link = withScheme(url)
                if (!isWebLink(link)) {
                    toast(linkInvalid)
                } else {
                    busy = true
                    scope.launch {
                        try {
                            val via = Core.openUrl(node, link)
                            url = ""
                            toast(if (via == "queued") linkQueued else linkSent)
                        } catch (e: Exception) { toast(friendlyError(e)) } finally { busy = false }
                    }
                }
            }, enabled = !busy && url.isNotBlank())
        }
    }
}

/** Buttons for the actions the device listed in its status. */
@Composable
private fun Controls(node: String, st: DeviceStatus) {
    val toast = LocalToast.current
    val scope = rememberCoroutineScope()
    val run = { name: String, args: JSONObject ->
        scope.launch {
            try { Core.runAction(node, name, args) } catch (e: Exception) { toast(friendlyError(e)) }
        }
        Unit
    }
    @Composable
    fun Btn(label: Int, name: String, args: JSONObject, m: Modifier = Modifier) =
        GhostButton(stringResource(label), { run(name, args) }, m, small = true)

    Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
        if ("flashlight" in st.actions) {
            val on = st.torch == true
            SoftButton(
                stringResource(if (on) R.string.ctl_torch_off else R.string.ctl_torch_on),
                { run("flashlight", JSONObject().put("on", !on)) }, Modifier.fillMaxWidth(),
            )
        }
        if ("volume" in st.actions || "mute" in st.actions) {
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.CenterVertically) {
                Text(
                    st.volume?.let { stringResource(R.string.ctl_volume_level, it) } ?: stringResource(R.string.ctl_volume),
                    color = T.text2, style = MaterialTheme.typography.labelMedium, modifier = Modifier.weight(1f).announce(),
                )
                if ("volume" in st.actions) {
                    Btn(R.string.ctl_volume_down, "volume", JSONObject().put("step", -1))
                    Btn(R.string.ctl_volume_up, "volume", JSONObject().put("step", 1))
                }
                if ("mute" in st.actions) {
                    Btn(if (st.muted == true) R.string.ctl_unmute else R.string.ctl_mute, "mute", JSONObject().put("on", st.muted != true))
                }
            }
        }
        if ("media" in st.actions) {
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                Btn(R.string.ctl_previous, "media", JSONObject().put("key", "previous"), Modifier.weight(1f))
                Btn(R.string.ctl_play_pause, "media", JSONObject().put("key", "play_pause"), Modifier.weight(1f))
                Btn(R.string.ctl_next, "media", JSONObject().put("key", "next"), Modifier.weight(1f))
            }
        }
        if ("slides" in st.actions) {
            Muted(
                when (st.slidesNeeds) {
                    "accessibility" -> stringResource(R.string.ctl_slides_needs_accessibility)
                    "xdotool", "wtype" -> stringResource(R.string.ctl_slides_needs_tool)
                    else -> stringResource(R.string.ctl_slides)
                }
            )
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                Btn(R.string.ctl_slide_previous, "slides", JSONObject().put("key", "previous"), Modifier.weight(1f))
                Btn(R.string.ctl_slide_black, "slides", JSONObject().put("key", "black"), Modifier.weight(1f))
                Btn(R.string.ctl_slide_next, "slides", JSONObject().put("key", "next"), Modifier.weight(1f))
            }
        }
        if ("keep_awake" in st.actions) {
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.CenterVertically) {
                Text(
                    stringResource(R.string.ctl_awake), color = T.text2,
                    style = MaterialTheme.typography.labelMedium, modifier = Modifier.weight(1f),
                )
                Btn(R.string.ctl_awake_30, "keep_awake", JSONObject().put("minutes", 30))
                Btn(R.string.ctl_awake_60, "keep_awake", JSONObject().put("minutes", 60))
                Btn(R.string.ctl_awake_off, "keep_awake", JSONObject().put("minutes", 0))
            }
        }
        if ("search" in st.actions) {
            var query by remember { mutableStateOf("") }
            Field(query, { query = it }, stringResource(R.string.ctl_search_hint), Modifier.fillMaxWidth())
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                listOf("youtube" to R.string.ctl_search_youtube, "spotify" to R.string.ctl_search_spotify, "web" to R.string.ctl_search_web)
                    .forEach { (app, label) ->
                        GhostButton(stringResource(label), {
                            if (query.isNotBlank()) run("search", JSONObject().put("app", app).put("query", query.trim()))
                        }, Modifier.weight(1f), small = true)
                    }
            }
        }
        if ("lock" in st.actions) {
            Btn(R.string.ctl_lock, "lock", JSONObject(), Modifier.fillMaxWidth())
        }
        if ("ringer" in st.actions) {
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                listOf("normal" to R.string.ctl_ring_normal, "vibrate" to R.string.ctl_ring_vibrate, "silent" to R.string.ctl_ring_silent)
                    .forEach { (mode, label) ->
                        // The current mode is the filled one, and says "selected".
                        val m = Modifier.weight(1f)
                        if (st.ringer == mode) SoftButton(stringResource(label), { }, m, selected = true)
                        else Btn(label, "ringer", JSONObject().put("mode", mode), m.semantics { selected = false })
                    }
            }
        }
    }
}

/** "example.com/x" -> "https://example.com/x"; a link with a scheme is kept. */
private fun withScheme(input: String): String {
    val t = input.trim()
    return if (t.contains("://")) t else "https://$t"
}

private fun isWebLink(url: String): Boolean {
    val lower = url.lowercase()
    return (lower.startsWith("https://") || lower.startsWith("http://")) && url.length > "https://".length
}
