package com.mydomain.android.ui

import android.text.format.Formatter
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import com.mydomain.android.R
import com.mydomain.android.core.Core
import com.mydomain.android.core.DeviceKinds
import com.mydomain.android.core.DlItem
import com.mydomain.android.core.RemoteDownloadPlan
import com.mydomain.android.core.RemoteDownloadPlan.Line
import com.mydomain.android.core.RemoteDownloads
import com.mydomain.android.core.friendlyError
import com.mydomain.android.ffi.RemoteDownloadPrefs
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/*
 * "Download on <device>" on the phone: the device chooser for a shared
 * link, the question when another device asks this phone to download one,
 * the lines its transfer rows say, and its settings tile.
 */

/** Under a transfer's name, for a link download: "from example.com ·
 *  requested by Pixel" (here) or "on Laptop · from example.com". */
@Composable
fun dlHeadLine(here: Boolean, node: String, host: String): String =
    if (here) stringResource(R.string.dl_head_here, host, Core.peerName(node))
    else stringResource(R.string.dl_head_there, Core.peerName(node), host)

/** What an active link download is doing now, null when the core hasn't
 *  said (a plain progress line is shown then). */
@Composable
fun dlActiveLine(id: String): String? {
    val item = RemoteDownloads.byId[id] ?: return null
    if (item.finished) return null
    val device = Core.peerName(item.node)
    return when (RemoteDownloadPlan.line(item)) {
        Line.Checking -> stringResource(R.string.dl_line_checking, item.host)
        Line.Asking -> stringResource(R.string.dl_line_asking)
        Line.Queued -> stringResource(R.string.dl_line_queued)
        Line.Downloading -> stringResource(R.string.dl_line_downloading, item.host, device)
        Line.Requested -> stringResource(R.string.dl_line_requested, device)
        Line.WaitingThere -> stringResource(R.string.dl_line_waiting_there, device)
        Line.QueuedThere -> stringResource(R.string.dl_line_queued_there, device)
        Line.DownloadingThere -> stringResource(R.string.dl_line_downloading_there, device, item.host)
    }
}

/** "Download on…": the devices that download links, for
 *  [RemoteDownloads.pendingLink]. */
@Composable
fun DownloadOnChooser() {
    val link = RemoteDownloads.pendingLink ?: return
    val scope = rememberCoroutineScope()
    val toast = LocalToast.current
    val peers = Core.peers
    // Who says they download links: asked off the main thread.
    val targets by produceState<List<String>?>(null, link, peers) {
        value = withContext(Dispatchers.IO) {
            RemoteDownloadPlan.targets(
                peers.map { it.nodeId },
                supports = RemoteDownloads::supported,
                linked = { Core.links.containsKey(it) },
                name = { Core.peerName(it) },
            )
        }
    }
    val asked = stringResource(R.string.dl_asked)
    AlertDialog(
        onDismissRequest = { RemoteDownloads.pendingLink = null },
        containerColor = T.bar,
        title = { Text(stringResource(R.string.dl_choose_title), color = T.text) },
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                Muted(link)
                val list = targets
                when {
                    list == null -> Muted(stringResource(R.string.dl_choose_loading))
                    list.isEmpty() -> Muted(stringResource(R.string.dl_choose_none))
                    else -> list.forEach { node ->
                        val name = Core.peerName(node)
                        SoftButton(name, {
                            RemoteDownloads.pendingLink = null
                            scope.launch {
                                runCatching { RemoteDownloads.request(node, link) }.fold(
                                    { toast(String.format(asked, name)) },
                                    { toast(friendlyError(it)) },
                                )
                            }
                        }, Modifier.fillMaxWidth(), deviceIcon = DeviceKinds.iconOf(node))
                    }
                }
                Muted(stringResource(R.string.dl_choose_hint))
            }
        },
        confirmButton = {},
        dismissButton = {
            TextButton({ RemoteDownloads.pendingLink = null }) { Text(stringResource(R.string.cancel), color = T.text2) }
        },
    )
}

/** Another device asks this phone to download a link: allow it once,
 *  always from that device, or not. The oldest question first. */
@Composable
fun DownloadAskDialog() {
    val ask: DlItem = RemoteDownloads.asks.firstOrNull() ?: return
    val ctx = LocalContext.current
    val scope = rememberCoroutineScope()
    val toast = LocalToast.current
    val device = Core.peerName(ask.node)
    val answer = { allow: Boolean, always: Boolean ->
        scope.launch {
            runCatching { RemoteDownloads.answer(ask.id, allow, always) }.onFailure { toast(friendlyError(it)) }
        }
        com.mydomain.android.DownloadAsk.dismiss(ctx, ask.id)
    }
    AlertDialog(
        onDismissRequest = {},
        containerColor = T.bar,
        title = { Text(stringResource(R.string.dl_ask_title, device), color = T.text) },
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                Text(ask.name, color = T.text)
                Muted(
                    if (ask.size > 0) stringResource(R.string.dl_ask_from_size, ask.host, Formatter.formatShortFileSize(ctx, ask.size))
                    else stringResource(R.string.dl_ask_from, ask.host),
                )
                Muted(ask.url)
                Muted(stringResource(R.string.dl_ask_hint))
                SoftButton(stringResource(R.string.dl_ask_once), { answer(true, false) }, Modifier.fillMaxWidth())
                SoftButton(stringResource(R.string.dl_ask_always, device), { answer(true, true) }, Modifier.fillMaxWidth())
            }
        },
        confirmButton = {},
        dismissButton = {
            TextButton({ answer(false, false) }) { Text(stringResource(R.string.dl_ask_deny), color = T.text2) }
        },
    )
}

/** Settings: "Downloads from my devices". */
@OptIn(ExperimentalLayoutApi::class)
@Composable
fun RemoteDownloadSettingsTile() {
    val scope = rememberCoroutineScope()
    val toast = LocalToast.current
    var prefs by remember { mutableStateOf<RemoteDownloadPrefs?>(null) }
    LaunchedEffect(Unit) { prefs = runCatching { RemoteDownloads.settings() }.getOrNull() }
    val save = { next: RemoteDownloadPrefs ->
        prefs = next
        scope.launch { runCatching { RemoteDownloads.setSettings(next) }.onFailure { toast(friendlyError(it)) } }
    }
    Tile(title = stringResource(R.string.dl_settings_title)) {
        Muted(stringResource(R.string.dl_settings_hint))
        val p = prefs ?: return@Tile
        val policy = RemoteDownloadPlan.policy(p.policy)
        FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            listOf(
                "ask" to R.string.dl_policy_ask,
                "always" to R.string.dl_policy_always,
                "never" to R.string.dl_policy_never,
            ).forEach { (id, label) ->
                SoftButton(stringResource(label), { save(p.copy(policy = id)) }, selected = policy == id)
            }
        }
        SwitchRow(stringResource(R.string.dl_settings_lan), p.allowLan) { save(p.copy(allowLan = it)) }
        Muted(stringResource(R.string.dl_settings_max))
        val max = RemoteDownloadPlan.nearestMax(p.maxMb)
        FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            RemoteDownloadPlan.MAX_CHOICES.forEach { mb ->
                val label = when {
                    mb == 0L -> stringResource(R.string.dl_max_none)
                    mb >= 1024 -> stringResource(R.string.dl_max_gb, (mb / 1024).toInt())
                    else -> stringResource(R.string.dl_max_mb, mb.toInt())
                }
                SoftButton(label, { save(p.copy(maxMb = mb)) }, selected = max == mb)
            }
        }
        if (p.allowed.isNotEmpty()) {
            Muted(stringResource(R.string.dl_settings_allowed, p.allowed.joinToString(", ") { Core.peerName(it) }))
            GhostButton(stringResource(R.string.dl_settings_forget), { save(p.copy(allowed = emptyList())) }, small = true)
        }
        Muted(stringResource(R.string.dl_settings_login_hint))
    }
}
