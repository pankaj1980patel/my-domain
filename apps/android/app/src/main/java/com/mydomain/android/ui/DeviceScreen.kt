package com.mydomain.android.ui

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
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import com.mydomain.android.core.Core
import com.mydomain.android.core.LinkState
import com.mydomain.android.core.friendlyError
import com.mydomain.android.core.linkStateOf
import com.mydomain.android.core.niceName
import com.mydomain.android.core.progressLabel
import kotlinx.coroutines.launch

/** One device: its status, then a tile per feature. */
@Composable
fun DeviceScreen(node: String, onBack: () -> Unit) {
    val peer = Core.peers.firstOrNull { it.nodeId == node }
    val toast = LocalToast.current
    val scope = rememberCoroutineScope()

    Column(Modifier.fillMaxSize().imePadding()) {
        Row(Modifier.fillMaxWidth().padding(start = 4.dp, top = 4.dp), verticalAlignment = Alignment.CenterVertically) {
            IconButton(onClick = onBack) { Icon(Icons.AutoMirrored.Filled.ArrowBack, "Back", tint = T.text) }
            Text("Your devices", style = MaterialTheme.typography.bodyMedium, color = T.text2)
        }
        if (peer == null) {
            EmptyState("Not in your list", "This device isn't in your list any more.")
            return
        }
        val s = linkStateOf(node)
        LazyColumn(
            Modifier.fillMaxSize(),
            contentPadding = PaddingValues(start = 16.dp, end = 16.dp, bottom = 16.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            item {
                Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.padding(vertical = 4.dp)) {
                    Avatar(peer.name, 56.dp)
                    Spacer(Modifier.width(14.dp))
                    Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(3.dp)) {
                        Text(niceName(peer.name), style = MaterialTheme.typography.headlineSmall, color = T.text)
                        StatusPill(node)
                        if (s == LinkState.Connecting || s == LinkState.Failed) {
                            progressLabel(Core.progress[node])?.let { Muted(it) }
                        }
                    }
                    if (s == LinkState.Offline || s == LinkState.Failed) {
                        PrimaryButton("Connect", {
                            scope.launch { try { Core.connect(node) } catch (e: Exception) { toast(friendlyError(e)) } }
                        })
                    }
                }
            }
            item { ChatTile(node) }
            item { ClipboardTile(node) }
            item { NotifyTile() }
            item { AppsTile(node) }
        }
    }
}

@Composable
private fun ChatTile(node: String) {
    val toast = LocalToast.current
    val scope = rememberCoroutineScope()
    var text by remember { mutableStateOf("") }
    var busy by remember { mutableStateOf(false) }
    val thread = Core.messages.filter { it.node == node }.takeLast(50)

    Tile(title = "Messages") {
        if (thread.isEmpty()) {
            Muted("Say hi. Messages go straight to this device, encrypted.", Modifier.fillMaxWidth().padding(vertical = 8.dp), center = true)
        }
        thread.forEach { m ->
            Box(Modifier.fillMaxWidth(), contentAlignment = if (m.incoming) Alignment.CenterStart else Alignment.CenterEnd) {
                Text(
                    m.text,
                    Modifier.widthIn(max = 280.dp)
                        .background(if (m.incoming) T.surface2 else T.accent, RoundedCornerShape(14.dp))
                        .padding(horizontal = 12.dp, vertical = 8.dp),
                    color = if (m.incoming) T.text else T.accentText,
                    style = MaterialTheme.typography.bodyMedium,
                )
            }
        }
        Row(verticalAlignment = Alignment.CenterVertically) {
            Field(text, { text = it }, "Write a message", Modifier.weight(1f))
            Spacer(Modifier.width(8.dp))
            PrimaryButton("Send", {
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
    Tile(title = "Clipboard") {
        Muted("Copy something there, paste it here.")
        SoftButton(if (busy) "Getting it…" else "Get its clipboard", {
            busy = true
            scope.launch {
                try { Core.pullClipboard(node); toast("Asked for its clipboard") } catch (e: Exception) { toast(friendlyError(e)) } finally { busy = false }
            }
        }, Modifier.fillMaxWidth(), enabled = !busy)
        ClipSyncSwitch("Keep clipboards in sync")
    }
}

@Composable
private fun NotifyTile() {
    val toast = LocalToast.current
    val scope = rememberCoroutineScope()
    var title by remember { mutableStateOf("") }
    var body by remember { mutableStateOf("") }
    Tile(title = "Send a notification") {
        Field(title, { title = it }, "Title")
        Field(body, { body = it }, "Message (optional)")
        SoftButton("Send to all my devices", {
            val t = title.trim()
            scope.launch {
                try { Core.shareNotification(t, body.trim()); title = ""; body = ""; toast("Sent to your devices") } catch (e: Exception) { toast(friendlyError(e)) }
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
    Tile(title = "App notifications") {
        Muted("Choose which of its apps can show notifications here.")
        if (data == null) {
            SoftButton("Show its apps", {
                scope.launch { try { Core.requestApps(node) } catch (e: Exception) { toast(friendlyError(e)) } }
            })
        } else {
            Field(q, { q = it }, "Search apps")
            val shown = data.apps.filter { it.label.contains(q, ignoreCase = true) }.take(50)
            if (shown.isEmpty()) Muted("No apps to show.")
            shown.forEach { a ->
                SwitchRow(a.label, a.pkg in data.subscribed) {
                    scope.launch { try { Core.toggleApp(node, a.pkg) } catch (e: Exception) { toast(friendlyError(e)) } }
                }
            }
        }
    }
}
