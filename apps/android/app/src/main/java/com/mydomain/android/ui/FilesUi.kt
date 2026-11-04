package com.mydomain.android.ui

import android.text.format.Formatter
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.width
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.pluralStringResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import com.mydomain.android.FileActions
import com.mydomain.android.R
import com.mydomain.android.core.Core
import com.mydomain.android.core.Transfer
import com.mydomain.android.core.TransferState
import com.mydomain.android.core.friendlyError
import com.mydomain.android.core.niceName

/** Files: send some or a whole folder (system pickers), and this device's
 *  transfers. */
@Composable
fun FilesTile(node: String) {
    val toast = LocalToast.current
    var busy by remember { mutableStateOf(false) }
    // A picked folder being copied: files done and how many (null = not yet counted).
    var folderBusy by remember { mutableStateOf(false) }
    var copied by remember { mutableStateOf<Pair<Int, Int>?>(null) }
    val preparingFolder = stringResource(R.string.folder_preparing)
    val picker = rememberLauncherForActivityResult(ActivityResultContracts.OpenMultipleDocuments()) { uris ->
        if (uris.isEmpty()) return@rememberLauncherForActivityResult
        busy = true
        Core.sendFiles(node, uris, onDone = { busy = false }, onError = { toast(friendlyError(it)) })
    }
    val folderPicker = rememberLauncherForActivityResult(ActivityResultContracts.OpenDocumentTree()) { tree ->
        if (tree == null) return@rememberLauncherForActivityResult
        folderBusy = true
        toast(preparingFolder)
        Core.sendFolderAsync(
            node, tree,
            onProgress = { done, total -> copied = done to total },
            onSent = { n -> toast(Core.plural(R.plurals.folder_started, n, n, Core.peerName(node))) },
            onError = { toast(friendlyError(it)) },
            onDone = { folderBusy = false; copied = null },
        )
    }
    val list = Core.transfers.filter { it.node == node }.take(20)

    Tile(title = stringResource(R.string.files_title)) {
        Muted(stringResource(R.string.files_body))
        Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            SoftButton(
                stringResource(if (busy) R.string.files_preparing else R.string.files_send),
                { picker.launch(arrayOf("*/*")) },
                Modifier.weight(1f), enabled = !busy && !folderBusy,
            )
            SoftButton(
                copied?.let { (done, total) -> stringResource(R.string.folder_preparing_count, done, total) }
                    ?: stringResource(if (folderBusy) R.string.files_preparing else R.string.files_send_folder),
                { folderPicker.launch(null) },
                Modifier.weight(1f), enabled = !busy && !folderBusy,
            )
        }
        list.forEach { TransferRow(it) }
    }
}

@Composable
private fun TransferRow(t: Transfer) {
    val ctx = LocalContext.current
    val toast = LocalToast.current
    val noApp = stringResource(R.string.file_no_app)
    val size = { n: Long -> Formatter.formatShortFileSize(ctx, n) }
    val state = when (t.state) {
        TransferState.Active -> when {
            t.incoming -> stringResource(R.string.file_receiving, size(t.done), size(t.total))
            t.done == 0L && !Core.links.containsKey(t.node) -> stringResource(R.string.file_waiting)
            else -> stringResource(R.string.file_sending, size(t.done), size(t.total))
        }
        TransferState.Done -> stringResource(if (t.incoming) R.string.file_saved else R.string.file_sent)
        TransferState.Failed ->
            if (t.reason == "cancelled") stringResource(R.string.file_cancelled)
            else stringResource(R.string.file_failed, t.reason ?: "")
    }
    Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Column(Modifier.weight(1f)) {
                Text(
                    t.name, style = MaterialTheme.typography.bodyMedium, color = T.text,
                    maxLines = 1, overflow = TextOverflow.Ellipsis,
                )
                Text(
                    state, style = MaterialTheme.typography.labelMedium,
                    color = if (t.state == TransferState.Failed && t.reason != "cancelled") T.danger else T.text2,
                )
            }
            Spacer(Modifier.width(8.dp))
            when {
                t.state == TransferState.Active -> GhostButton(stringResource(R.string.cancel), {
                    Core.cancelFileAsync(t.id)
                }, small = true)
                t.state == TransferState.Done && t.incoming && t.uri != null -> GhostButton(stringResource(R.string.file_open), {
                    if (!FileActions.open(ctx, t.uri, t.mime)) toast(noApp)
                }, small = true)
            }
        }
        if (t.state == TransferState.Active) {
            LinearProgressIndicator(
                progress = { t.fraction },
                modifier = Modifier.fillMaxWidth(),
                color = T.accent,
                trackColor = T.surface2,
            )
        }
    }
}

/** Files or text shared from another app: pick the device that gets them. */
@Composable
fun ShareChooser() {
    val uris = Core.pendingShare
    val text = Core.pendingText.takeIf { uris.isEmpty() }
    if (uris.isEmpty() && text == null) return
    val toast = LocalToast.current
    val peers = Core.peers
    val sendingTo = stringResource(R.string.share_sending)
    AlertDialog(
        onDismissRequest = { Core.clearShare() },
        containerColor = T.bar,
        title = { Text(stringResource(R.string.share_title), color = T.text) },
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                Muted(if (text != null) stringResource(R.string.share_text_body) else pluralStringResource(R.plurals.share_body, uris.size, uris.size))
                if (peers.isEmpty()) Muted(stringResource(R.string.share_no_devices))
                peers.forEach { p ->
                    val name = niceName(p.name)
                    SoftButton(name, {
                        Core.clearShare()
                        toast(String.format(sendingTo, name))
                        if (text != null) Core.sendSharedTextAsync(p.nodeId, text, onError = { toast(friendlyError(it)) })
                        else Core.sendFiles(p.nodeId, uris, onError = { toast(friendlyError(it)) })
                    }, Modifier.fillMaxWidth())
                }
            }
        },
        confirmButton = {},
        dismissButton = {
            TextButton({ Core.clearShare() }) { Text(stringResource(R.string.cancel), color = T.text2) }
        },
    )
}
