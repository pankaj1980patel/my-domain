package com.mydomain.android.ui

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.ExtendedFloatingActionButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.pluralStringResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import com.mydomain.android.PhoneAutomations
import com.mydomain.android.R
import com.mydomain.android.core.ActivityLog
import com.mydomain.android.core.ActivityQuery
import com.mydomain.android.core.ActivityRow
import com.mydomain.android.core.ChatHistory
import com.mydomain.android.core.Core
import com.mydomain.android.core.Health
import com.mydomain.android.core.Notes
import com.mydomain.android.core.ReadLater
import com.mydomain.android.core.Timers
import com.mydomain.android.core.TransferLog
import com.mydomain.android.core.timeAgo

// The redesign's tab and hub screens: Devices (a tab), More (a tab), and the
// hubs More opens: Files & transfers, Messages, Workspace. Each only gathers
// screens and tiles that already exist; nothing here is a new feature.

/** Devices: All / Online / Offline, a row per device (art, name, status,
 *  battery), and "Add a device" (the join invite, as in Settings). */
@Composable
fun DevicesScreen(onOpen: (String) -> Unit) {
    var filter by rememberSaveable { mutableStateOf("all") }
    var adding by rememberSaveable { mutableStateOf(false) }
    val all = sortedPeers()
    val online = all.filter { isOnline(it) }
    val offline = all.filter { !isOnline(it) }
    val list = when (filter) {
        "online" -> online
        "offline" -> offline
        else -> all
    }
    FollowLinksMap()
    Box(Modifier.fillMaxSize()) {
        LazyColumn(Modifier.fillMaxSize(), contentPadding = PaddingValues(start = 16.dp, end = 16.dp, top = 8.dp, bottom = 96.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
            item {
                PageHead(
                    stringResource(R.string.tab_devices),
                    if (all.isEmpty()) stringResource(R.string.home_subtitle_empty)
                    else pluralStringResource(R.plurals.home_online_of, all.size, online.size, all.size),
                )
            }
            item {
                Pills(
                    listOf(
                        Pill("all", stringResource(R.string.devices_all, all.size)),
                        Pill("online", stringResource(R.string.devices_online, online.size)),
                        Pill("offline", stringResource(R.string.devices_offline, offline.size)),
                    ),
                    filter, { filter = it },
                )
            }
            if (adding) item { SoftCard { AddDevicePanel(onClose = { adding = false }) } }
            if (all.isEmpty()) {
                item { EmptyState(stringResource(R.string.home_empty_title), stringResource(R.string.home_empty_body)) }
            } else if (list.isEmpty()) {
                item { Muted(stringResource(R.string.devices_none_here), Modifier.padding(8.dp)) }
            } else {
                item {
                    SoftCard(padding = PaddingValues(4.dp)) {
                        list.forEach { n -> DeviceListRow(n) { onOpen(n) } }
                    }
                }
            }
        }
        if (!adding) {
            ExtendedFloatingActionButton(
                onClick = { adding = true },
                modifier = Modifier.align(Alignment.BottomEnd).padding(16.dp),
                containerColor = T.primary,
                contentColor = T.onPrimary,
                icon = { LIcon(R.drawable.ic_lucide_plus, T.onPrimary, 22.dp) },
                text = { Text(stringResource(R.string.join_add_device), fontWeight = FontWeight.Bold) },
            )
        }
    }
}

/** Where More's rows go. */
enum class MoreItem { Files, Messages, Workspace, Notes, Timers, ReadLater, Automations, Search, Connections, Health, Settings, Help }

/** More: everything that isn't a tab, as rows with a chevron. */
@Composable
fun MoreScreen(onItem: (MoreItem) -> Unit) {
    val activeTransfers = TransferLog.list.count { !it.finished }
    val notes = Notes.byId.size
    val unread = ReadLater.unread()
    val running = Timers.sorted().count { it.state == "running" }
    val rules = PhoneAutomations.items.size
    LazyColumn(Modifier.fillMaxSize(), contentPadding = ScreenPadding, verticalArrangement = Arrangement.spacedBy(12.dp)) {
        item { PageHead(stringResource(R.string.tab_more)) }
        item {
            RowGroup {
                NavRow(
                    R.drawable.ic_lucide_arrow_left_right, Hues.files, stringResource(R.string.more_files),
                    if (activeTransfers > 0) stringResource(R.string.transfers_open_active, activeTransfers) else stringResource(R.string.more_files_sub),
                    onClick = { onItem(MoreItem.Files) },
                )
                NavRow(
                    R.drawable.ic_lucide_message_circle, Hues.messages, stringResource(R.string.more_messages),
                    stringResource(R.string.more_messages_sub), onClick = { onItem(MoreItem.Messages) },
                )
                NavRow(
                    R.drawable.ic_lucide_layers, Hue.Cyan, stringResource(R.string.more_workspace),
                    stringResource(R.string.more_workspace_sub), onClick = { onItem(MoreItem.Workspace) },
                )
            }
        }
        item {
            RowGroup {
                NavRow(
                    R.drawable.ic_lucide_sticky_note, Hue.Amber, stringResource(R.string.notes_title),
                    if (notes > 0) pluralStringResource(R.plurals.more_notes_count, notes, notes) else stringResource(R.string.notes_subtitle),
                    onClick = { onItem(MoreItem.Notes) },
                )
                NavRow(
                    R.drawable.ic_lucide_timer, Hue.Amber, stringResource(R.string.timers_title),
                    if (running > 0) pluralStringResource(R.plurals.timers_running, running, running) else stringResource(R.string.timers_entry_hint),
                    onClick = { onItem(MoreItem.Timers) },
                )
                NavRow(
                    R.drawable.ic_lucide_bookmark, Hue.Cyan, stringResource(R.string.read_later_title),
                    if (unread > 0) pluralStringResource(R.plurals.read_later_unread_count, unread, unread) else stringResource(R.string.read_later_none_unread),
                    onClick = { onItem(MoreItem.ReadLater) },
                )
            }
        }
        item {
            RowGroup {
                NavRow(
                    R.drawable.ic_lucide_zap, Hue.Amber, stringResource(R.string.automations_title),
                    if (rules > 0) pluralStringResource(R.plurals.more_rules_count, rules, rules) else stringResource(R.string.more_automations_sub),
                    onClick = { onItem(MoreItem.Automations) },
                )
                NavRow(
                    R.drawable.ic_lucide_search, Hue.Blue, stringResource(R.string.more_search),
                    stringResource(R.string.more_search_sub), onClick = { onItem(MoreItem.Search) },
                )
                NavRow(
                    R.drawable.ic_lucide_network, Hue.Green, stringResource(R.string.more_connections),
                    stringResource(R.string.map_entry_body), onClick = { onItem(MoreItem.Connections) },
                )
                NavRow(
                    R.drawable.ic_lucide_shield_check, if (Health.bad > 0) Hue.Red else Hue.Green, stringResource(R.string.more_security_health),
                    if (Health.bad > 0) stringResource(R.string.health_summary_bad, Health.bad) else stringResource(R.string.more_health_sub),
                    onClick = { onItem(MoreItem.Health) },
                )
                NavRow(
                    R.drawable.ic_lucide_settings, Hue.Muted, stringResource(R.string.settings_title),
                    stringResource(R.string.more_settings_sub), onClick = { onItem(MoreItem.Settings) },
                )
                NavRow(
                    R.drawable.ic_lucide_circle_help, Hue.Cyan, stringResource(R.string.help_title),
                    stringResource(R.string.help_settings_hint), onClick = { onItem(MoreItem.Help) },
                )
            }
        }
    }
}

/** The Files hub's tabs. */
enum class FilesTab(val key: String) { Files("files"), Transfers("transfers"), Sync("sync") }

/** Files & transfers: send to a device or browse its folders (Files), every
 *  transfer (Transfers, the old Transfers screen), and synced folders
 *  (Sync, the same tile as in Settings). */
@Composable
fun FilesHubScreen(
    tab: FilesTab,
    onTab: (FilesTab) -> Unit,
    onBack: () -> Unit,
    onOpenDevice: (String, String?) -> Unit,
    foundTransfer: String? = null,
) {
    var target by remember { mutableStateOf<String?>(null) }
    val send = rememberFileSender { target }
    Column(Modifier.fillMaxSize()) {
        BackBar(stringResource(R.string.more_files), onBack)
        Pills(
            listOf(
                Pill(FilesTab.Files, stringResource(R.string.files_title), R.drawable.ic_lucide_folder),
                Pill(FilesTab.Transfers, stringResource(R.string.transfers_title), R.drawable.ic_lucide_arrow_left_right),
                Pill(FilesTab.Sync, stringResource(R.string.files_tab_sync), R.drawable.ic_lucide_refresh_cw),
            ),
            tab, onTab, Modifier.padding(horizontal = 16.dp, vertical = 4.dp),
        )
        when (tab) {
            FilesTab.Transfers -> TransfersScreen(onBack, onOpenDevice = { onOpenDevice(it, null) }, found = foundTransfer, embedded = true)
            FilesTab.Sync -> Column(
                Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(16.dp),
                verticalArrangement = Arrangement.spacedBy(12.dp),
            ) { SyncFoldersTile() }
            FilesTab.Files -> LazyColumn(Modifier.fillMaxSize(), contentPadding = ScreenPadding, verticalArrangement = Arrangement.spacedBy(12.dp)) {
                item { Muted(stringResource(R.string.files_hub_hint), Modifier.padding(horizontal = 4.dp)) }
                val peers = sortedPeers()
                if (peers.isEmpty()) item { EmptyState(stringResource(R.string.home_empty_title), stringResource(R.string.home_empty_body)) }
                peers.forEach { n ->
                    item(key = n) {
                        val st = Core.deviceStatus[n]
                        GroupCard(
                            R.drawable.ic_lucide_folder, Hues.files, Core.peerName(n), null,
                            onHeader = { onOpenDevice(n, "tab:" + DeviceTab.Files.key) },
                            actions = listOfNotNull(
                                TileAction(R.drawable.ic_lucide_send, stringResource(R.string.dev_act_send), Hues.files, !send.busy) {
                                    target = n
                                    send.files()
                                },
                                TileAction(R.drawable.ic_lucide_folder_open, stringResource(R.string.dev_act_send_folder), Hues.files, !send.busy) {
                                    target = n
                                    send.folder()
                                },
                                if (st?.sharedFolders != null) {
                                    TileAction(R.drawable.ic_lucide_folder, stringResource(R.string.dev_act_browse), Hues.files) {
                                        onOpenDevice(n, "tab:" + DeviceTab.Files.key)
                                    }
                                } else {
                                    null
                                },
                            ),
                        )
                    }
                }
            }
        }
    }
}

/** The Messages hub's tabs. */
enum class MessagesTab { Chats, Notifications }

/** The newest message with [node]: this session's, else the kept history's. */
private fun lastMessage(node: String): Pair<String, Long>? {
    val live = Core.messages.filter { it.node == node }.maxByOrNull { it.ts }
    val kept = if (ChatHistory.enabled) ChatHistory.page(node, null, 1).entries.lastOrNull() else null
    return when {
        live != null && (kept == null || live.ts >= kept.at) -> live.text to live.ts
        kept != null -> kept.text to kept.at
        else -> null
    }
}

/** Messages: a chat per device (Chats), and notifications (Notifications:
 *  send one to all your devices, and the recent ones from Activity). */
@Composable
fun MessagesHubScreen(
    tab: MessagesTab,
    onTab: (MessagesTab) -> Unit,
    onBack: () -> Unit,
    onOpenDevice: (String, String?) -> Unit,
    onActivityRow: (ActivityRow) -> Unit,
) {
    val ctx = LocalContext.current
    Column(Modifier.fillMaxSize()) {
        BackBar(stringResource(R.string.more_messages), onBack)
        Pills(
            listOf(
                Pill(MessagesTab.Chats, stringResource(R.string.msg_tab_chats), R.drawable.ic_lucide_message_circle),
                Pill(MessagesTab.Notifications, stringResource(R.string.msg_tab_notifications), R.drawable.ic_lucide_bell),
            ),
            tab, onTab, Modifier.padding(horizontal = 16.dp, vertical = 4.dp),
        )
        LazyColumn(Modifier.fillMaxSize(), contentPadding = ScreenPadding, verticalArrangement = Arrangement.spacedBy(12.dp)) {
            when (tab) {
                MessagesTab.Chats -> {
                    val peers = sortedPeers().sortedByDescending { lastMessage(it)?.second ?: 0L }
                    if (peers.isEmpty()) item { EmptyState(stringResource(R.string.home_empty_title), stringResource(R.string.home_empty_body)) }
                    else item {
                        SoftCard(padding = PaddingValues(4.dp)) {
                            peers.forEach { n -> ChatRow(n) { onOpenDevice(n, "tab:" + DeviceTab.Communication.key) } }
                        }
                    }
                }
                MessagesTab.Notifications -> {
                    item { NotifyTile() }
                    item { SectionTitle(stringResource(R.string.msg_recent_notifications)) }
                    item {
                        val sources = listOf(ActivityLog.version, Core.peers)
                        val rows = remember(sources) {
                            ActivityLog.page(ActivityQuery(kinds = setOf("notifications"), limit = 30)).items
                        }
                        SoftCard(padding = PaddingValues(8.dp)) {
                            if (rows.isEmpty()) Muted(stringResource(R.string.msg_no_notifications), Modifier.padding(8.dp))
                            rows.forEach { a -> ActivityRowView(ctx, a, onActivityRow, last = a === rows.last()) }
                        }
                    }
                }
            }
        }
    }
}

/** A device's chat as a row: its art, name, the newest message and when. */
@Composable
private fun ChatRow(node: String, onOpen: () -> Unit) {
    val last = lastMessage(node)
    Row(
        Modifier.fillMaxWidth().heightIn(min = 68.dp).clip(RoundedCornerShape(16.dp))
            .clickable(role = Role.Button, onClick = onOpen).padding(horizontal = 12.dp, vertical = 8.dp)
            .semantics(mergeDescendants = true) {},
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Box(Modifier.width(56.dp), contentAlignment = Alignment.Center) { DeviceArt(node, 44.dp, maxWidth = 56.dp) }
        Spacer(Modifier.width(12.dp))
        Column(Modifier.weight(1f)) {
            Text(Core.peerName(node), style = MaterialTheme.typography.titleSmall, color = T.text, fontWeight = FontWeight.SemiBold, maxLines = 1, overflow = TextOverflow.Ellipsis)
            Text(
                last?.first ?: stringResource(R.string.chat_empty), style = MaterialTheme.typography.bodySmall,
                color = T.text2, maxLines = 1, overflow = TextOverflow.Ellipsis,
            )
        }
        last?.second?.let {
            Spacer(Modifier.width(8.dp))
            Text(timeAgo(it), style = MaterialTheme.typography.labelSmall, color = T.text3)
        }
        Spacer(Modifier.width(6.dp))
        Chevron()
    }
}

/** Workspace: Notes, Timers and Read later as cards, each opening its screen. */
@Composable
fun WorkspaceScreen(onBack: () -> Unit, onNotes: () -> Unit, onTimers: () -> Unit, onReadLater: () -> Unit) {
    val notes = Notes.byId.size
    val unread = ReadLater.unread()
    val saved = ReadLater.byId.size
    val running = Timers.sorted().count { it.state == "running" }
    Column(Modifier.fillMaxSize()) {
        BackBar(stringResource(R.string.more_workspace), onBack)
        Column(
            Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(start = 16.dp, end = 16.dp, bottom = 16.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            Muted(stringResource(R.string.more_workspace_sub), Modifier.padding(horizontal = 4.dp))
            SoftCard(onClick = onNotes, padding = PaddingValues(6.dp)) {
                NavRow(
                    R.drawable.ic_lucide_sticky_note, Hue.Amber, stringResource(R.string.notes_title),
                    if (notes > 0) pluralStringResource(R.plurals.more_notes_count, notes, notes) else stringResource(R.string.notes_subtitle),
                )
            }
            SoftCard(onClick = onTimers, padding = PaddingValues(6.dp)) {
                NavRow(
                    R.drawable.ic_lucide_timer, Hue.Green, stringResource(R.string.timers_title),
                    if (running > 0) pluralStringResource(R.plurals.timers_running, running, running) else stringResource(R.string.timers_entry_hint),
                )
            }
            SoftCard(onClick = onReadLater, padding = PaddingValues(6.dp)) {
                NavRow(
                    R.drawable.ic_lucide_bookmark, Hue.Violet, stringResource(R.string.read_later_title),
                    when {
                        unread > 0 -> pluralStringResource(R.plurals.read_later_unread_count, unread, unread)
                        saved > 0 -> stringResource(R.string.read_later_none_unread)
                        else -> stringResource(R.string.more_read_later_sub)
                    },
                )
            }
        }
    }
}
