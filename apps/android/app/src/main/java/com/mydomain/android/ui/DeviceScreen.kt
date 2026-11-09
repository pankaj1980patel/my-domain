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

/** The device page's tabs (pill tabs under the header, pinned while the
 *  page scrolls). Each tile of the page lives on one ([TILE_TAB]);
 *  Overview has a short row of quick actions; Settings has what it is,
 *  how it's linked, Verify and, kept apart at the bottom, Remove.
 *  The tiles are in DeviceTiles.kt, DeviceCommTiles.kt and
 *  DeviceControlTiles.kt; Arrange in DeviceArrange.kt. */
enum class DeviceTab(val key: String, val label: Int, val icon: Int) {
    Overview("overview", R.string.dev_tab_overview, R.drawable.ic_lucide_layout_grid),
    Communication("comm", R.string.dev_tab_comm, R.drawable.ic_lucide_message_circle),
    Files("files", R.string.dev_tab_files, R.drawable.ic_lucide_folder),
    Remote("remote", R.string.dev_tab_remote, R.drawable.ic_lucide_mouse_pointer_2),
    Media("media", R.string.dev_tab_media, R.drawable.ic_lucide_music),
    Settings("settings", R.string.settings_title, R.drawable.ic_lucide_settings),
    ;

    companion object {
        fun of(key: String?): DeviceTab? = entries.firstOrNull { it.key == key }
    }
}

/** Which tab each tile is on (ids as in [deviceTiles]). Clipboard is the
 *  last part of Files. */
private val TILE_TAB = mapOf(
    "chat" to DeviceTab.Communication,
    "notify" to DeviceTab.Communication,
    "computer-notifs" to DeviceTab.Communication,
    "apps" to DeviceTab.Communication,
    "files" to DeviceTab.Files,
    "browse" to DeviceTab.Files,
    "clipboard" to DeviceTab.Files,
    "clipboard-history" to DeviceTab.Files,
    "control" to DeviceTab.Remote,
    "control-sound" to DeviceTab.Remote,
    "control-screen" to DeviceTab.Remote,
    "control-open" to DeviceTab.Remote,
    "touchpad" to DeviceTab.Remote,
    "screen-share" to DeviceTab.Remote,
    "commands" to DeviceTab.Remote,
    "now-playing" to DeviceTab.Media,
    "phone-audio" to DeviceTab.Media,
    "computer-audio" to DeviceTab.Media,
    "info" to DeviceTab.Settings,
    "connection" to DeviceTab.Settings,
)

private fun tabOf(id: String): DeviceTab = TILE_TAB[id] ?: DeviceTab.Overview

/** The tab you were last on for each device (like the desktop's). */
private const val LAST_TAB = "deviceTab."

/** One device: Back (its name once the header has scrolled away) and a
 *  menu; its art, status and when it was last seen; pinned pill tabs
 *  (Overview, Messages, Files, Control, Media, Settings). [focus]: what to
 *  bring up: "msg:<id>" a message, "clip:<id>" a clip (marked),
 *  "tab:<key>" a tab, "tile:<id>" a tile, "edit" the name editor. */
@OptIn(ExperimentalFoundationApi::class)
@Composable
fun DeviceScreen(node: String, onBack: () -> Unit, focus: String? = null, onHealth: () -> Unit = {}) {
    val peer = Core.peers.firstOrNull { it.nodeId == node }
    val ctx = LocalContext.current
    val prefs = remember { ctx.getSharedPreferences("md", Context.MODE_PRIVATE) }
    val toast = LocalToast.current
    val scope = rememberCoroutineScope()
    var editing by rememberSaveable(node, focus) { mutableStateOf(focus == "edit") }
    var menu by remember { mutableStateOf(false) }
    val list = rememberLazyListState()
    val foundMessage = focus?.takeIf { it.startsWith("msg:") }?.removePrefix("msg:")?.toLongOrNull()
    val foundClip = focus?.takeIf { it.startsWith("clip:") }?.removePrefix("clip:")?.toLongOrNull()
    val focusTile = focus?.takeIf { it.startsWith("tile:") }?.removePrefix("tile:")
    val startTab = when {
        foundMessage != null -> DeviceTab.Communication
        foundClip != null -> DeviceTab.Files
        focusTile != null -> tabOf(focusTile)
        focus?.startsWith("tab:") == true -> DeviceTab.of(focus.removePrefix("tab:")) ?: DeviceTab.Overview
        else -> DeviceTab.of(prefs.getString(LAST_TAB + node, null)) ?: DeviceTab.Overview
    }
    var tabKey by rememberSaveable(node, focus) { mutableStateOf(startTab.key) }
    // A tile to bring into view once its tab shows (a card's action, a search).
    var scrollTo by remember(node, focus) {
        mutableStateOf(
            when {
                foundMessage != null -> "chat"
                foundClip != null -> "clipboard-history"
                focusTile != null -> focusTile
                focus == "edit" -> "label"
                else -> null
            },
        )
    }
    val goTo = { t: DeviceTab, tile: String? ->
        tabKey = t.key
        scrollTo = tile
        prefs.edit().putString(LAST_TAB + node, t.key).apply()
    }
    LaunchedEffect(tabKey, scrollTo) {
        val k = scrollTo ?: return@LaunchedEffect
        list.scrollToKey(k)
        scrollTo = null
    }
    FollowLinksMap()
    // Arrange (a long press on a tile, or the menu): the screen's tiles in
    // the saved order, for this kind of device or this device alone
    // (PageLayoutStore).
    var arranging by rememberSaveable(node) { mutableStateOf(false) }
    var said by remember(node) { mutableStateOf("") }
    // Back leaves Arrange or the name editor first, not the page.
    BackHandler(enabled = arranging || editing) {
        arranging = false
        editing = false
    }
    val kind = pageKind(node)
    val name = Core.peerName(node)
    // The tiles are made once per device (not on every status update);
    // what the status changes is read below (PageTile.available, content).
    val tiles = remember(node, name, foundMessage, foundClip, kind) {
        deviceTiles(node, name, foundMessage, foundClip).filter { kind in it.kinds }
    }
    val tileIds = remember(tiles) { tiles.map { it.id } }
    val layout = PageLayoutStore.view(kind, node, tileIds)
    val arrangeLabel = stringResource(R.string.layout_arrange)
    val send = rememberFileSender { node }
    // The name shows in the bar once the header has scrolled away.
    val scrolled by remember { derivedStateOf { list.firstVisibleItemIndex > 0 } }

    Column(Modifier.fillMaxSize()) {
        BackBar(
            when {
                peer == null -> stringResource(R.string.dev_card_info)
                scrolled -> name
                else -> ""
            },
            onBack,
        ) {
            if (peer != null) {
                Box {
                    IconButton(onClick = { menu = true }) {
                        LIcon(R.drawable.ic_lucide_ellipsis_vertical, T.text, 24.dp, stringResource(R.string.dev_menu, name))
                    }
                    DeviceMenu(
                        node, menu, onDismiss = { menu = false },
                        onRename = { editing = true; scrollTo = "label" },
                        onArrange = { arranging = true; said = "" },
                    )
                }
            }
        }
        if (peer == null) {
            EmptyState(stringResource(R.string.device_not_found_title), stringResource(R.string.device_not_found_body))
            return
        }
        val s = linkStateOf(node)
        val st = Core.deviceStatus[node]
        // Tabs with nothing on them for this device are left out.
        val byId = remember(tiles) { tiles.associateBy { it.id } }
        val shown = layout.shown.mapNotNull { byId[it] }.filter { it.available(st) }
        val tabs = DeviceTab.entries.filter { t ->
            t == DeviceTab.Overview || t == DeviceTab.Settings || shown.any { tabOf(it.id) == t }
        }
        // A tab left with nothing (a tile hidden, the device changed): Overview.
        val tab = DeviceTab.of(tabKey)?.takeIf { it in tabs } ?: DeviceTab.Overview
        LazyColumn(
            Modifier.fillMaxSize(),
            state = list,
            contentPadding = PaddingValues(start = 16.dp, end = 16.dp, bottom = 24.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            item(key = "hero") {
                Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        ArtAvatar(node, 80.dp)
                        Spacer(Modifier.width(14.dp))
                        Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(4.dp)) {
                            Row(verticalAlignment = Alignment.CenterVertically) {
                                LabelDot(node)
                                Text(
                                    name, style = MaterialTheme.typography.headlineSmall, color = T.text,
                                    maxLines = 2, overflow = TextOverflow.Ellipsis,
                                    modifier = Modifier.semantics { heading() },
                                )
                            }
                            // What it is: "Apple MacBook Pro", "Samsung Galaxy S23".
                            DeviceKinds.describe(node).takeIf { it.isNotEmpty() }?.let { Muted(it) }
                            StatusPill(node)
                            (viaText(node) ?: lastSeenText(node))?.let { Muted(it) }
                            if (s == LinkState.Connecting || s == LinkState.Failed) {
                                progressLabel(Core.progress[node])?.let { Muted(it, Modifier.announce()) }
                            }
                        }
                    }
                    FactChips(node)
                    // Asleep? Wake it (Wake-on-LAN), if it said it may be woken.
                    WakeIt(node)
                    if (s == LinkState.Offline || s == LinkState.Failed) {
                        PrimaryButton(stringResource(R.string.connect), {
                            scope.launch { try { Core.connect(node) } catch (e: Exception) { toast(friendlyError(e)) } }
                        }, Modifier.fillMaxWidth())
                    }
                    // About the whole device: its security key changed (refused
                    // until you decide); the health check found something red.
                    KeyChangeBanner(node)
                    HealthDeviceBanner(node, onHealth)
                }
            }
            if (!arranging) {
                stickyHeader(key = "tabs") {
                    Box(Modifier.fillMaxWidth().background(T.bg).padding(vertical = 4.dp)) {
                        Pills(tabs.map { Pill(it, stringResource(it.label), it.icon) }, tab, { goTo(it, null) })
                    }
                }
            }
            if (editing) item(key = "label") { DeviceLabelEditor(node) { editing = false } }
            if (arranging) {
                arrangeItems(node, name, kind, tiles, st, layout, said, { said = it }) { arranging = false }
            } else {
                if (tab == DeviceTab.Overview) {
                    item(key = "cards") {
                        OverviewActions(node, name, st, shown.map { it.id }.toSet(), send, goTo)
                    }
                }
                val here = shown.filter { tabOf(it.id) == tab }
                for (t in here) item(key = t.id) {
                    Box(Modifier.longPressToArrange(node, arrangeLabel) { arranging = true }) { t.content(st) }
                }
                if (layout.shown.isEmpty()) item { Muted(stringResource(R.string.layout_all_hidden)) }
                if (tab == DeviceTab.Settings) {
                    // Compare security keys with it; "Verified" once you have.
                    item(key = "verify") { Tile(title = stringResource(R.string.dev_security)) { VerifyRow(node) } }
                    // Remove it for good (asks first; "Allow again" in Settings),
                    // apart at the very bottom.
                    item(key = "remove") {
                        Column {
                            Spacer(Modifier.height(24.dp))
                            Tile(title = stringResource(R.string.dev_remove_title)) {
                                Muted(stringResource(R.string.dev_remove_hint))
                                RemoveDeviceRow(node, name, onBack)
                            }
                        }
                    }
                }
            }
        }
    }
}

/** The ⋮ menu: rename, arrange, connect / reconnect / disconnect. */
@Composable
private fun DeviceMenu(
    node: String,
    open: Boolean,
    onDismiss: () -> Unit,
    onRename: () -> Unit,
    onArrange: () -> Unit,
) {
    val toast = LocalToast.current
    val scope = rememberCoroutineScope()
    val s = linkStateOf(node)
    val run = { block: suspend () -> Unit ->
        onDismiss()
        scope.launch { try { block() } catch (e: Exception) { toast(friendlyError(e)) } }
        Unit
    }
    DropdownMenu(open, onDismissRequest = onDismiss, containerColor = T.bar) {
        @Composable
        fun Item(icon: Int, label: Int, onClick: () -> Unit) = DropdownMenuItem(
            text = { Text(stringResource(label), color = T.text) },
            leadingIcon = { LIcon(icon, T.text2, 20.dp) },
            onClick = onClick,
        )
        Item(R.drawable.ic_lucide_pencil, R.string.dev_rename) { onDismiss(); onRename() }
        Item(R.drawable.ic_lucide_layers, R.string.layout_arrange) { onDismiss(); onArrange() }
        if (s == LinkState.Direct || s == LinkState.Relay) {
            // Connected: drop it, or drop it and connect afresh.
            Item(R.drawable.ic_lucide_refresh_cw, R.string.reconnect) { run { Core.reconnect(node) } }
            Item(R.drawable.ic_lucide_power, R.string.disconnect) { run { Core.disconnect(node) } }
        } else {
            Item(R.drawable.ic_lucide_link_2, R.string.connect) { run { Core.connect(node) } }
        }
    }
}

/** Overview: one short row of the quick actions (as Home's): message it,
 *  send it a file, get its clipboard, ring it, and see or control its
 *  screen when it can. The full controls are on the tabs above. */
@Composable
private fun OverviewActions(
    node: String,
    name: String,
    st: DeviceStatus?,
    shown: Set<String>,
    send: FileSender,
    goTo: (DeviceTab, String?) -> Unit,
) {
    val toast = LocalToast.current
    val scope = rememberCoroutineScope()
    val ringing = stringResource(R.string.find_ringing, name)
    val ringQueued = stringResource(R.string.find_ring_queued, name)
    val asked = stringResource(R.string.clipboard_asked)
    val sending = stringResource(R.string.dev_sending)
    // Messages, clipboard asks and rings wait for a link (the core queues
    // them), so these work offline too.
    val actions = listOfNotNull(
        if ("chat" in shown) {
            TileAction(R.drawable.ic_lucide_message_circle, stringResource(R.string.dev_act_chat), Hues.messages) {
                goTo(DeviceTab.Communication, "chat")
            }
        } else {
            null
        },
        TileAction(R.drawable.ic_lucide_send, stringResource(R.string.home_q_send), Hues.files, !send.busy, sending) { send.files() },
        TileAction(R.drawable.ic_lucide_clipboard, stringResource(R.string.home_q_clipboard), Hues.files) {
            scope.launch { try { Core.pullClipboard(node); toast(asked) } catch (e: Exception) { toast(friendlyError(e)) } }
        },
        TileAction(R.drawable.ic_lucide_bell_ring, stringResource(R.string.home_q_ring), Hues.alerts) {
            scope.launch {
                try { toast(if (Core.ring(node) == "queued") ringQueued else ringing) } catch (e: Exception) { toast(friendlyError(e)) }
            }
        },
        when {
            "screen-share" in shown || "touchpad" in shown -> TileAction(R.drawable.ic_lucide_monitor, stringResource(R.string.home_q_screen), Hues.tools) {
                goTo(DeviceTab.Remote, if ("screen-share" in shown) "screen-share" else "touchpad")
            }
            st != null && "locate" in st.actions -> TileAction(R.drawable.ic_lucide_map_pin, stringResource(R.string.dev_act_find), Hues.tools) {
                goTo(DeviceTab.Remote, "control")
            }
            else -> null
        },
    )
    SoftCard(padding = PaddingValues(horizontal = 6.dp, vertical = 10.dp)) {
        Row(Modifier.fillMaxWidth()) {
            actions.forEach { a -> QuickAction(a, Modifier.weight(1f)) }
        }
    }
}

/** "phone" or "computer": which layout [node]'s screen uses. Its own report
 *  first (its status, what kind of device it said it is), then its icon; a
 *  computer when nothing says. */
private fun pageKind(node: String): String {
    Core.deviceStatus[node]?.platform?.let { return if (it == "android") "phone" else "computer" }
    DeviceKinds.kinds[node]?.os?.takeIf { it.isNotEmpty() }?.let {
        return if (it == "android" || it == "ios") "phone" else "computer"
    }
    val icon = DeviceLabels.labels[node]?.icon ?: DeviceKinds.kinds[node]?.icon
    return if (DeviceIconShapes.isHandheld(icon)) "phone" else "computer"
}
