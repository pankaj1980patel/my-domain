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
 *  Overview has the grouped action cards; Settings has what it is, how
 *  it's linked, Verify and, kept apart at the bottom, Remove. */
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
    var editing by remember(node, focus) { mutableStateOf(focus == "edit") }
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
    var arranging by remember(node) { mutableStateOf(false) }
    var said by remember(node) { mutableStateOf("") }
    // Back leaves Arrange or the name editor first, not the page.
    BackHandler(enabled = arranging || editing) {
        arranging = false
        editing = false
    }
    val kind = pageKind(node)
    val tiles = deviceTiles(node, Core.peerName(node), foundMessage, foundClip).filter { kind in it.kinds }
    val layout = PageLayoutStore.view(kind, node, tiles.map { it.id })
    val arrangeLabel = stringResource(R.string.layout_arrange)
    val send = rememberFileSender { node }
    val name = Core.peerName(node)
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
                        onHealth = onHealth,
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
        val shown = layout.shown.mapNotNull { id -> tiles.firstOrNull { it.id == id } }.filter { it.available }
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
                arrangeItems(node, name, kind, tiles, layout, said, { said = it }) { arranging = false }
            } else {
                if (tab == DeviceTab.Overview) {
                    item(key = "cards") {
                        OverviewCards(node, name, st, shown.map { it.id }.toSet(), tabs, send, goTo)
                    }
                }
                val here = shown.filter { tabOf(it.id) == tab }
                for (t in here) item(key = t.id) {
                    Box(Modifier.longPressToArrange(node, arrangeLabel) { arranging = true }) { t.content() }
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

/** The ⋮ menu: rename, arrange, connect / reconnect / disconnect, and the
 *  health check. */
@Composable
private fun DeviceMenu(
    node: String,
    open: Boolean,
    onDismiss: () -> Unit,
    onRename: () -> Unit,
    onArrange: () -> Unit,
    onHealth: () -> Unit,
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
        Item(R.drawable.ic_lucide_shield_check, R.string.dev_health_check) { onDismiss(); onHealth() }
    }
}

/** Overview's grouped cards: Messaging & notifications, Files & sharing
 *  (clipboard last), Device tools (only what it can do), and Device info.
 *  Each tile acts, or opens the tab with the full controls. [shown]: the
 *  tiles it offers now. */
@Composable
private fun OverviewCards(
    node: String,
    name: String,
    st: DeviceStatus?,
    shown: Set<String>,
    tabs: List<DeviceTab>,
    send: FileSender,
    goTo: (DeviceTab, String?) -> Unit,
) {
    val toast = LocalToast.current
    val scope = rememberCoroutineScope()
    val ringing = stringResource(R.string.find_ringing, name)
    val ringQueued = stringResource(R.string.find_ring_queued, name)
    val asked = stringResource(R.string.clipboard_asked)
    val sending = stringResource(R.string.dev_sending)
    Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
        val comm = buildList {
            if ("chat" in shown) add(TileAction(R.drawable.ic_lucide_message_circle, stringResource(R.string.dev_act_chat), Hues.messages) { goTo(DeviceTab.Communication, "chat") })
            val notif = when {
                "computer-notifs" in shown -> "computer-notifs"
                "apps" in shown -> "apps"
                "notify" in shown -> "notify"
                else -> null
            }
            if (notif != null) add(TileAction(R.drawable.ic_lucide_bell, stringResource(R.string.dev_act_notifications), Hues.alerts) { goTo(DeviceTab.Communication, notif) })
        }
        if (DeviceTab.Communication in tabs) {
            GroupCard(
                R.drawable.ic_lucide_message_circle, Hues.messages, stringResource(R.string.dev_card_comm),
                stringResource(R.string.dev_card_comm_sub), { goTo(DeviceTab.Communication, null) }, comm,
            )
        }
        val files = buildList {
            add(TileAction(R.drawable.ic_lucide_send, stringResource(R.string.dev_act_send), Hues.files, !send.busy, sending) { send.files() })
            add(TileAction(R.drawable.ic_lucide_folder_open, stringResource(R.string.dev_act_send_folder), Hues.files, !send.busy, sending) { send.folder() })
            if ("browse" in shown) add(TileAction(R.drawable.ic_lucide_folder, stringResource(R.string.dev_act_browse), Hues.files) { goTo(DeviceTab.Files, "browse") })
            // The clipboard ask waits for a link (the core queues it).
            add(
                TileAction(R.drawable.ic_lucide_clipboard, stringResource(R.string.dev_act_get_clipboard), Hues.files) {
                    scope.launch { try { Core.pullClipboard(node); toast(asked) } catch (e: Exception) { toast(friendlyError(e)) } }
                },
            )
        }
        GroupCard(
            R.drawable.ic_lucide_folder, Hues.files, stringResource(R.string.dev_card_files),
            stringResource(R.string.dev_card_files_sub), { goTo(DeviceTab.Files, null) }, files,
        )
        val tools = buildList {
            add(
                TileAction(R.drawable.ic_lucide_bell_ring, stringResource(R.string.dev_act_ring), Hues.alerts) {
                    scope.launch {
                        try {
                            val via = Core.ring(node)
                            toast(if (via == "queued") ringQueued else ringing)
                        } catch (e: Exception) { toast(friendlyError(e)) }
                    }
                },
            )
            if (st != null && "locate" in st.actions) add(TileAction(R.drawable.ic_lucide_map_pin, stringResource(R.string.dev_act_find), Hues.tools) { goTo(DeviceTab.Remote, "control") })
            if ("control-sound" in shown || "control-screen" in shown) {
                add(TileAction(R.drawable.ic_lucide_sliders_horizontal, stringResource(R.string.dev_act_controls), Hues.tools) { goTo(DeviceTab.Remote, if ("control-sound" in shown) "control-sound" else "control-screen") })
            }
            if ("screen-share" in shown) add(TileAction(R.drawable.ic_lucide_screen_share, stringResource(R.string.dev_act_screen), Hues.tools) { goTo(DeviceTab.Remote, "screen-share") })
            if ("touchpad" in shown) add(TileAction(R.drawable.ic_lucide_mouse_pointer_2, stringResource(R.string.touchpad_title), Hues.tools) { goTo(DeviceTab.Remote, "touchpad") })
            if ("commands" in shown) add(TileAction(R.drawable.ic_lucide_terminal, stringResource(R.string.run_title), Hues.tools) { goTo(DeviceTab.Remote, "commands") })
        }
        GroupCard(
            R.drawable.ic_lucide_wrench, Hues.tools, stringResource(R.string.dev_card_tools),
            stringResource(R.string.dev_card_tools_sub), if (DeviceTab.Remote in tabs) ({ goTo(DeviceTab.Remote, null) }) else null, tools,
        )
        // Device info: what it is, its OS and address; the rest is on its
        // Settings tab.
        val facts = listOfNotNull(
            osName(node),
            DeviceKinds.kinds[node]?.model?.takeIf { it.isNotEmpty() },
            Core.peers.firstOrNull { it.nodeId == node }?.ip?.takeIf { it.isNotEmpty() },
        )
        SoftCard(padding = PaddingValues(6.dp)) {
            NavRow(
                R.drawable.ic_lucide_info, Hues.plain, stringResource(R.string.dev_card_info),
                facts.joinToString(" · ").ifEmpty { null },
                onClick = { goTo(DeviceTab.Settings, if ("info" in shown) "info" else null) },
            )
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
        // Clipboard: the last part of Files.
        PageTile("clipboard", R.string.clipboard_title) { ClipboardTile(node) },
        // Text sent to it and from it as clipboard, kept on this phone.
        PageTile("clipboard-history", R.string.clip_history_title, available = ClipLog.enabled) {
            ClipHistoryTile(node, name, foundClip)
        },
        // Find & control, in parts (the "control" id stays: saved layouts).
        PageTile("control", R.string.ctl_group_find) { FindTile(node, name) },
        PageTile("control-sound", R.string.ctl_group_sound, available = st != null && st.actions.any { it in SOUND }) {
            st?.let { SoundTile(node, it) }
        },
        PageTile("control-screen", R.string.ctl_group_screen, available = st != null && st.actions.any { it in SCREEN }) {
            st?.let { ScreenPowerTile(node, it) }
        },
        PageTile("control-open", R.string.ctl_group_open) { OpenOnTile(node, name) },
        // What it said about itself: battery, storage, memory, processor, uptime.
        PageTile("info", R.string.dev_card_info, available = st != null && hasInfo(st)) {
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
        // What plays there, with play/pause, skip, seek and volume when its
        // app takes media commands (media_ctl); an older one just shows it.
        PageTile(
            "now-playing", R.string.media_title,
            available = st?.mediaCtl == true || Core.nowPlaying[node] != null,
        ) { MediaTile(node, name) },
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

    Tile {
        // The title, with "Clear" kept small at its end, away from the input.
        Row(verticalAlignment = Alignment.CenterVertically) {
            CardTitle(stringResource(R.string.chat_title), Modifier.weight(1f))
            if (thread.isNotEmpty()) {
                TextButton({ Core.clearChat(node) }, Modifier.heightIn(min = 48.dp)) {
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
        // Coordinates in plain digits with a point, whatever the language.
        Text(
            stringResource(
                R.string.locate_where,
                String.format(Locale.ROOT, "%.5f", place.lat), String.format(Locale.ROOT, "%.5f", place.lon), place.acc.toInt(), mins,
            ),
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

/** Actions of the "Sound & media" part, and of "Screen & power". */
private val SOUND = setOf("volume", "mute", "ringer", "media")
private val SCREEN = setOf("flashlight", "keep_awake", "lock", "slides")

/** Run one of the device's own actions, saying why if it fails. */
@Composable
private fun rememberRun(node: String): (String, JSONObject) -> Unit {
    val toast = LocalToast.current
    val scope = rememberCoroutineScope()
    return { name, args ->
        scope.launch {
            try { Core.runAction(node, name, args) } catch (e: Exception) { toast(friendlyError(e)) }
        }
    }
}

/** A row of equal buttons for [node]'s action, under its own label line. */
@Composable
private fun Btn(label: Int, run: () -> Unit, m: Modifier = Modifier) =
    GhostButton(stringResource(label), run, m, small = true)

/** Find: ring it, and where it is (when it lets your devices ask). Asks
 *  for its status when the page opens over a live link. */
@Composable
private fun FindTile(node: String, name: String) {
    val toast = LocalToast.current
    val scope = rememberCoroutineScope()
    val status = Core.deviceStatus[node]
    val ringing = stringResource(R.string.find_ringing, name)
    val ringQueued = stringResource(R.string.find_ring_queued, name)

    // Ask for its status when the screen opens; the reply fills `status`
    // (and the info tile). Asked only over a live link (and again when one
    // comes up): asking a device that isn't connected would wake it by
    // push just to show this.
    val linked = Core.links.containsKey(node)
    LaunchedEffect(node, linked) { if (linked) runCatching { Core.requestStatus(node) } }

    Tile(title = stringResource(R.string.ctl_group_find)) {
        Muted(stringResource(R.string.find_body))
        SoftButton(stringResource(R.string.find_ring), {
            scope.launch {
                try {
                    val via = Core.ring(node)
                    toast(if (via == "queued") ringQueued else ringing)
                } catch (e: Exception) { toast(friendlyError(e)) }
            }
        }, Modifier.fillMaxWidth())
        if (status != null && "locate" in status.actions) LocateRow(node, name, status)
    }
}

/** Sound & media: volume, mute, the ringer and the media keys, each only
 *  when the device listed it. */
@Composable
private fun SoundTile(node: String, st: DeviceStatus) {
    val run = rememberRun(node)
    Tile(title = stringResource(R.string.ctl_group_sound)) {
        if ("volume" in st.actions || "mute" in st.actions) {
            Muted(st.volume?.let { stringResource(R.string.ctl_volume_level, it) } ?: stringResource(R.string.ctl_volume), Modifier.announce())
            Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                if ("volume" in st.actions) {
                    Btn(R.string.ctl_volume_down, { run("volume", JSONObject().put("step", -1)) }, Modifier.weight(1f))
                    Btn(R.string.ctl_volume_up, { run("volume", JSONObject().put("step", 1)) }, Modifier.weight(1f))
                }
                if ("mute" in st.actions) {
                    Btn(
                        if (st.muted == true) R.string.ctl_unmute else R.string.ctl_mute,
                        { run("mute", JSONObject().put("on", st.muted != true)) }, Modifier.weight(1f),
                    )
                }
            }
        }
        if ("ringer" in st.actions) {
            Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                listOf("normal" to R.string.ctl_ring_normal, "vibrate" to R.string.ctl_ring_vibrate, "silent" to R.string.ctl_ring_silent)
                    .forEach { (mode, label) ->
                        // The current mode is the filled one, and says "selected".
                        val m = Modifier.weight(1f)
                        if (st.ringer == mode) SoftButton(stringResource(label), { }, m, selected = true)
                        else Btn(label, { run("ringer", JSONObject().put("mode", mode)) }, m.semantics { selected = false })
                    }
            }
        }
        if ("media" in st.actions) {
            Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                Btn(R.string.ctl_previous, { run("media", JSONObject().put("key", "previous")) }, Modifier.weight(1f))
                Btn(R.string.ctl_play_pause, { run("media", JSONObject().put("key", "play_pause")) }, Modifier.weight(1f))
                Btn(R.string.ctl_next, { run("media", JSONObject().put("key", "next")) }, Modifier.weight(1f))
            }
        }
    }
}

/** Screen & power: the flashlight, keeping it awake, locking it and the
 *  slide remote, each only when the device listed it. */
@Composable
private fun ScreenPowerTile(node: String, st: DeviceStatus) {
    val run = rememberRun(node)
    Tile(title = stringResource(R.string.ctl_group_screen)) {
        if ("flashlight" in st.actions) {
            val on = st.torch == true
            SoftButton(
                stringResource(if (on) R.string.ctl_torch_off else R.string.ctl_torch_on),
                { run("flashlight", JSONObject().put("on", !on)) }, Modifier.fillMaxWidth(),
            )
        }
        if ("keep_awake" in st.actions) {
            Muted(stringResource(R.string.ctl_awake))
            Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                Btn(R.string.ctl_awake_30, { run("keep_awake", JSONObject().put("minutes", 30)) }, Modifier.weight(1f))
                Btn(R.string.ctl_awake_60, { run("keep_awake", JSONObject().put("minutes", 60)) }, Modifier.weight(1f))
                Btn(R.string.ctl_awake_off, { run("keep_awake", JSONObject().put("minutes", 0)) }, Modifier.weight(1f))
            }
        }
        if ("lock" in st.actions) {
            Btn(R.string.ctl_lock, { run("lock", JSONObject()) }, Modifier.fillMaxWidth())
        }
        if ("slides" in st.actions) {
            Muted(
                when (st.slidesNeeds) {
                    "accessibility" -> stringResource(R.string.ctl_slides_needs_accessibility)
                    "xdotool", "wtype" -> stringResource(R.string.ctl_slides_needs_tool)
                    else -> stringResource(R.string.ctl_slides)
                },
            )
            Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                Btn(R.string.ctl_slide_previous, { run("slides", JSONObject().put("key", "previous")) }, Modifier.weight(1f))
                Btn(R.string.ctl_slide_black, { run("slides", JSONObject().put("key", "black")) }, Modifier.weight(1f))
                Btn(R.string.ctl_slide_next, { run("slides", JSONObject().put("key", "next")) }, Modifier.weight(1f))
            }
        }
    }
}

/** Open on it: a web link, and a search in an app when it can. */
@Composable
private fun OpenOnTile(node: String, name: String) {
    val toast = LocalToast.current
    val scope = rememberCoroutineScope()
    val run = rememberRun(node)
    val st = Core.deviceStatus[node]
    var url by remember { mutableStateOf("") }
    var busy by remember { mutableStateOf(false) }
    val linkSent = stringResource(R.string.find_link_sent, name)
    val linkQueued = stringResource(R.string.find_link_queued, name)
    val linkInvalid = stringResource(R.string.find_link_invalid)
    Tile(title = stringResource(R.string.ctl_group_open)) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Field(url, { url = it }, stringResource(R.string.find_link_hint), Modifier.weight(1f))
            Spacer(Modifier.width(8.dp))
            SoftButton(stringResource(R.string.send), {
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
        if (st != null && "search" in st.actions) {
            var query by remember { mutableStateOf("") }
            Field(query, { query = it }, stringResource(R.string.ctl_search_hint), Modifier.fillMaxWidth())
            Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                listOf("youtube" to R.string.ctl_search_youtube, "spotify" to R.string.ctl_search_spotify, "web" to R.string.ctl_search_web)
                    .forEach { (app, label) ->
                        Btn(label, {
                            if (query.isNotBlank()) run("search", JSONObject().put("app", app).put("query", query.trim()))
                        }, Modifier.weight(1f))
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
