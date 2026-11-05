package com.mydomain.android.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.pluralStringResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import com.mydomain.android.R
import com.mydomain.android.SyncFolders
import com.mydomain.android.core.Core
import com.mydomain.android.core.friendlyError
import com.mydomain.android.core.timeAgo
import com.mydomain.android.ffi.SyncPairInfo
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/**
 * Settings → Synced folders: every folder this phone keeps the same with a
 * computer (how it stands, Pause / Sync now / Remove), and a computer's
 * invitation to sync one (Accept: choose where it goes; Decline). The core
 * syncs them while NetService runs.
 */
@Composable
fun SyncFoldersTile() {
    LaunchedEffect(Unit) { withContext(Dispatchers.IO) { Core.refreshSync() } }
    Tile(title = stringResource(R.string.sync_title)) {
        Muted(stringResource(R.string.sync_hint))
        if (Core.syncPairs.isEmpty()) {
            Muted(stringResource(R.string.sync_empty))
        }
        Core.syncPairs.forEachIndexed { i, p ->
            if (i > 0) HorizontalDivider(color = T.border)
            SyncPairRow(p)
        }
    }
}

@Composable
private fun modeLabel(mode: String): String = stringResource(
    when (mode) {
        "send" -> R.string.sync_mode_send
        "receive" -> R.string.sync_mode_receive
        else -> R.string.sync_mode_both
    },
)

/** What the pair is doing, in words. */
@Composable
private fun stateLine(p: SyncPairInfo, device: String): String = when (p.state) {
    "invited" -> stringResource(R.string.sync_state_invited)
    "pending" -> stringResource(R.string.sync_state_pending, device)
    "paused" -> stringResource(R.string.sync_state_paused)
    "error" -> stringResource(R.string.sync_state_error, p.error.orEmpty())
    "syncing" -> {
        val n = p.syncing.toInt().coerceAtLeast(1)
        pluralStringResource(R.plurals.sync_state_syncing, n, n)
    }
    "waiting" -> stringResource(R.string.sync_state_waiting, device)
    else -> stringResource(R.string.sync_state_in_sync)
}

@Composable
private fun SyncPairRow(p: SyncPairInfo) {
    val ctx = LocalContext.current
    val toast = LocalToast.current
    val scope = rememberCoroutineScope()
    var confirming by remember(p.id) { mutableStateOf(false) }
    var choosing by remember(p.id) { mutableStateOf(false) }
    val device = Core.peerName(p.peer)
    val act = { block: suspend () -> Unit ->
        scope.launch {
            try { block() } catch (e: Exception) { toast(friendlyError(e)) }
        }
    }
    val accepted = stringResource(R.string.sync_accepted, p.name, device)
    val removed = stringResource(R.string.sync_removed, p.name)

    Column(Modifier.fillMaxWidth(), verticalArrangement = Arrangement.spacedBy(6.dp)) {
        Text(p.name, style = MaterialTheme.typography.titleSmall, color = T.text)
        Text(
            stateLine(p, device), Modifier.announce(),
            style = MaterialTheme.typography.bodySmall,
            color = if (p.state == "error") T.danger else T.text2,
        )
        if (p.state == "invited") {
            Muted(stringResource(R.string.sync_invited_line, device, modeLabel(p.mode)))
            if (!choosing) {
                Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    PrimaryButton(stringResource(R.string.sync_accept), { choosing = true })
                    GhostButton(stringResource(R.string.sync_decline), { act { Core.syncRemove(p.id) } })
                }
            } else {
                Muted(stringResource(R.string.sync_where))
                val main = remember(p.id) { SyncFolders.destination(ctx, p.name) }
                val own = remember(p.id) { SyncFolders.destination(ctx, p.name, appOnly = true) }
                val accept = { dir: java.io.File ->
                    act {
                        Core.syncAccept(p.id, dir.absolutePath)
                        choosing = false
                        toast(accepted)
                    }
                }
                SoftButton(
                    stringResource(if (SyncFolders.publicPossible) R.string.sync_where_downloads else R.string.sync_where_app, main.name),
                    { accept(main) }, Modifier.fillMaxWidth(),
                )
                if (SyncFolders.publicPossible) {
                    SoftButton(stringResource(R.string.sync_where_app, own.name), { accept(own) }, Modifier.fillMaxWidth())
                }
                Muted(stringResource(R.string.sync_where_hint))
                GhostButton(stringResource(R.string.cancel), { choosing = false }, small = true)
            }
        } else {
            if (p.path.isNotEmpty()) Muted(p.path)
            Muted(details(p))
            val renamed = p.renamed.toInt()
            if (renamed > 0) {
                Muted(pluralStringResource(R.plurals.sync_renamed, renamed, renamed))
                Muted(stringResource(R.string.sync_renamed_hint))
            }
            if (!confirming) {
                Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    if (p.state != "pending") {
                        GhostButton(
                            stringResource(if (p.paused) R.string.sync_resume else R.string.sync_pause),
                            { act { Core.syncPause(p.id, !p.paused) } }, small = true,
                        )
                        if (!p.paused) {
                            GhostButton(stringResource(R.string.sync_now), { act { Core.syncNow(p.id) } }, small = true)
                        }
                    }
                    GhostButton(stringResource(R.string.sync_remove), { confirming = true }, small = true)
                }
            } else {
                Muted(stringResource(R.string.sync_remove_confirm, p.name, device))
                Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    PrimaryButton(stringResource(R.string.sync_remove), {
                        act {
                            Core.syncRemove(p.id)
                            confirming = false
                            toast(removed)
                        }
                    })
                    GhostButton(stringResource(R.string.cancel), { confirming = false })
                }
            }
        }
    }
}

/** "Both ways · 12 files · 1 conflict copy · in sync 2 min ago". */
@Composable
private fun details(p: SyncPairInfo): String {
    val parts = mutableListOf(modeLabel(p.mode))
    val files = p.files.toInt()
    if (files > 0) parts += pluralStringResource(R.plurals.sync_files, files, files)
    val conflicts = p.conflicts.toInt()
    if (conflicts > 0) parts += pluralStringResource(R.plurals.sync_conflicts, conflicts, conflicts)
    val skipped = p.skipped.toInt()
    if (skipped > 0) parts += pluralStringResource(R.plurals.sync_skipped, skipped, skipped)
    if (p.lastSynced > 0u) parts += stringResource(R.string.sync_last_synced, timeAgo(p.lastSynced.toLong() * 1000))
    return parts.joinToString(" · ")
}
