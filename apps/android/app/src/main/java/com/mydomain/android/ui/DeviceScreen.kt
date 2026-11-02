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
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import com.mydomain.android.R
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
            IconButton(onClick = onBack) { Icon(Icons.AutoMirrored.Filled.ArrowBack, stringResource(R.string.back), tint = T.text) }
            Text(stringResource(R.string.home_title), style = MaterialTheme.typography.bodyMedium, color = T.text2)
        }
        if (peer == null) {
            EmptyState(stringResource(R.string.device_not_found_title), stringResource(R.string.device_not_found_body))
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
                        PrimaryButton(stringResource(R.string.connect), {
                            scope.launch { try { Core.connect(node) } catch (e: Exception) { toast(friendlyError(e)) } }
                        })
                    }
                }
            }
            item { ChatTile(node) }
            item { FindTile(node, niceName(peer.name)) }
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

    Tile(title = stringResource(R.string.chat_title)) {
        if (thread.isEmpty()) {
            Muted(stringResource(R.string.chat_empty), Modifier.fillMaxWidth().padding(vertical = 8.dp), center = true)
        }
        thread.forEach { m ->
            val waiting = m.queued && m.pending
            Box(Modifier.fillMaxWidth(), contentAlignment = if (m.incoming) Alignment.CenterStart else Alignment.CenterEnd) {
                Column(
                    Modifier.widthIn(max = 280.dp)
                        .alpha(if (waiting) 0.7f else 1f)
                        .background(if (m.incoming) T.surface2 else T.accent, RoundedCornerShape(14.dp))
                        .padding(horizontal = 12.dp, vertical = 8.dp),
                ) {
                    Text(m.text, color = if (m.incoming) T.text else T.accentText, style = MaterialTheme.typography.bodyMedium)
                    if (waiting) {
                        Text(stringResource(R.string.chat_sends_when_connected), color = T.accentText, style = MaterialTheme.typography.labelMedium)
                    }
                }
            }
        }
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
        ClipSyncSwitch(stringResource(R.string.clipboard_sync))
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

/** Find & control: ring it, open a link on it, and its battery. */
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

    // Ask for its battery when the screen opens; the reply fills `status`.
    LaunchedEffect(node) { runCatching { Core.requestStatus(node) } }

    Tile(title = stringResource(R.string.find_title)) {
        val battery = status?.battery
        if (battery != null) {
            Text(
                stringResource(if (status.charging == true) R.string.find_battery_charging else R.string.find_battery, battery),
                style = MaterialTheme.typography.bodyMedium, color = T.text,
            )
        }
        Muted(stringResource(R.string.find_body))
        SoftButton(stringResource(R.string.find_ring), {
            scope.launch {
                try {
                    val via = Core.ring(node)
                    toast(if (via == "queued") ringQueued else ringing)
                } catch (e: Exception) { toast(friendlyError(e)) }
            }
        }, Modifier.fillMaxWidth())
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

/** "example.com/x" -> "https://example.com/x"; a link with a scheme is kept. */
private fun withScheme(input: String): String {
    val t = input.trim()
    return if (t.contains("://")) t else "https://$t"
}

private fun isWebLink(url: String): Boolean {
    val lower = url.lowercase()
    return (lower.startsWith("https://") || lower.startsWith("http://")) && url.length > "https://".length
}
