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
        val wakeSent = stringResource(R.string.wake_sent)
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
                        // Asleep? Wake it (Wake-on-LAN), if it ever said how.
                        if ((s == LinkState.Offline || s == LinkState.Failed) && Core.canWake(node)) {
                            GhostButton(stringResource(R.string.wake_it), {
                                scope.launch {
                                    try {
                                        Core.wake(node)
                                        toast(wakeSent)
                                    } catch (e: Exception) { toast(friendlyError(e)) }
                                }
                            }, small = true)
                        }
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
            item { ChatTile(node) }
            item { FilesTile(node) }
            item { FindTile(node, niceName(peer.name)) }
            // A computer that lets the phone control it (it lists "input").
            Core.deviceStatus[node]?.takeIf { "input" in it.actions }?.let { st ->
                item { TouchpadTile(node, st) }
            }
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
                        .background(if (m.incoming) T.surface2 else T.primary, MaterialTheme.shapes.medium)
                        .padding(horizontal = 12.dp, vertical = 8.dp),
                ) {
                    Text(m.text, color = if (m.incoming) T.text else T.onPrimary, style = MaterialTheme.typography.bodyMedium)
                    if (waiting) {
                        Text(stringResource(R.string.chat_sends_when_connected), color = T.onPrimary, style = MaterialTheme.typography.labelMedium)
                    }
                }
            }
        }
        if (thread.isNotEmpty()) {
            Box(Modifier.fillMaxWidth(), contentAlignment = Alignment.CenterEnd) {
                GhostButton(stringResource(R.string.chat_clear), { Core.clearChat(node) }, small = true)
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
        Text(stringResource(R.string.clipboard_recent), style = MaterialTheme.typography.labelLarge, color = T.text, modifier = Modifier.weight(1f))
        GhostButton(stringResource(R.string.clipboard_clear), { Core.clearClipHistory() }, small = true)
    }
    recent.forEach { e ->
        Row(verticalAlignment = Alignment.CenterVertically) {
            Column(Modifier.weight(1f)) {
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
    // Asked only over a live link (and again when one comes up): asking a
    // device that isn't connected would wake it by push just to show this.
    val linked = Core.links.containsKey(node)
    LaunchedEffect(node, linked) { if (linked) runCatching { Core.requestStatus(node) } }

    Tile(title = stringResource(R.string.find_title)) {
        val battery = status?.battery
        if (battery != null) {
            Text(
                stringResource(if (status.charging == true) R.string.find_battery_charging else R.string.find_battery, battery),
                style = MaterialTheme.typography.bodyMedium, color = T.text,
            )
        }
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
                    color = T.text2, style = MaterialTheme.typography.labelMedium, modifier = Modifier.weight(1f),
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
                        val m = Modifier.weight(1f)
                        if (st.ringer == mode) SoftButton(stringResource(label), { }, m)
                        else Btn(label, "ringer", JSONObject().put("mode", mode), m)
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
