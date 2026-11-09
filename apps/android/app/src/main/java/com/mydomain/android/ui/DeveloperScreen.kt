package com.mydomain.android.ui

import com.mydomain.android.R
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.heading
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.rememberScrollState
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.mydomain.android.core.ConnCheck
import com.mydomain.android.core.Core
import com.mydomain.android.core.friendlyError
import com.mydomain.android.core.timeAgo
import kotlinx.coroutines.launch
import androidx.compose.runtime.LaunchedEffect
import com.mydomain.android.core.ResourceLedger
import kotlinx.coroutines.delay

/** Hidden Developer screen: everything technical lives here, out of the way. */
@Composable
fun DeveloperScreen(onBack: () -> Unit) {
    // Not in release builds (it shows raw events and the core's insides):
    // whatever opened it, it closes at once. The crash log is in Settings.
    if (!com.mydomain.android.BuildConfig.DEBUG) {
        LaunchedEffect(Unit) { onBack() }
        return
    }
    val toast = LocalToast.current
    val scope = rememberCoroutineScope()
    var name by remember { mutableStateOf("") }
    var ip by remember { mutableStateOf("") }
    var port by remember { mutableStateOf("") }
    var filter by remember { mutableStateOf("") }
    var ledger by remember { mutableStateOf(ResourceLedger.book.snapshot()) }
    LaunchedEffect(Unit) {
        while (true) {
            Core.awaitOnScreen()
            ledger = ResourceLedger.book.snapshot()
            delay(2_000)
        }
    }

    fun run(block: suspend () -> Unit) = scope.launch {
        try { block() } catch (e: Exception) { toast(friendlyError(e)) }
    }

    LazyColumn(
        Modifier.fillMaxSize(),
        contentPadding = PaddingValues(start = 16.dp, end = 16.dp, bottom = 16.dp),
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        item {
            Row(Modifier.padding(top = 4.dp), verticalAlignment = Alignment.CenterVertically) {
                IconButton(onClick = onBack) { LIcon(R.drawable.ic_lucide_arrow_left, T.text, 24.dp, stringResource(R.string.back)) }
                Text("Developer", Modifier.semantics { heading() }, style = MaterialTheme.typography.headlineSmall, color = T.text)
            }
        }
        item {
            Tile(title = "This device") {
                Kv("Node", Core.me?.nodeId ?: "")
                Kv("LAN IP", Core.me?.ip ?: "")
                Kv("QUIC port", Core.me?.quicPort?.toString() ?: "")
                Kv("Server", Core.session?.serverUrl ?: "none")
                val engine by Core.engineState.collectAsState()
                Kv("Engine", engine.toString())
                Row(Modifier.horizontalScroll(rememberScrollState()), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    GhostButton("Firewall check", {
                        run {
                            val fs = Core.call { it.firewallCheck() }
                            toast(if (fs.inboundBlocked) "Inbound connections are blocked" else "Inbound connections are allowed")
                        }
                    }, small = true)
                    GhostButton("Refresh from server", { run { Core.refreshDevices(); toast("Refreshed") } }, small = true)
                    GhostButton("Scan LAN", { run { Core.call { it.scanLan() } } }, small = true)
                }
            }
        }
        // What holds the battery now (ResourceLedger): a lock or listener
        // that never goes away here is a leak.
        item {
            Tile(title = "Resource ledger") {
                if (ledger.isEmpty()) Muted("Nothing held.")
                ledger.forEach { e -> Text(ResourceLedger.book.describe(e), style = MaterialTheme.typography.bodySmall.copy(fontFamily = FontFamily.Monospace), color = T.text2) }
                Muted(if (Core.onScreen) "App on screen: screen loops run." else "App off screen: screen loops wait.")
            }
        }
        item {
            Tile(title = "Links") {
                if (Core.peers.isEmpty()) Muted("No devices.")
                Core.peers.forEach { p ->
                    Kv(p.name, "${Core.links[p.nodeId] ?: "—"} · ${p.ip}:${p.quicPort} · ${p.source}")
                }
            }
        }
        // The raw connection reports (a computer's Connection tile shows
        // them in plain words).
        if (ConnCheck.reports.isNotEmpty()) item {
            Tile(title = "Connection reports") {
                ConnCheck.reports.forEach { (node, r) ->
                    Text(Core.peers.find { it.nodeId == node }?.name ?: node, style = MaterialTheme.typography.labelMedium, color = T.text)
                    Text(r.json.toString(2), style = MaterialTheme.typography.bodySmall.copy(fontFamily = FontFamily.Monospace), color = T.text2)
                }
            }
        }
        item {
            Tile(title = "Add a device by address") {
                Field(name, { name = it }, "Name")
                Field(ip, { ip = it }, "IP address")
                Field(port, { port = it.filter(Char::isDigit) }, "QUIC port")
                SoftButton("Add", {
                    run {
                        Core.call { it.addManualPeer(name.ifBlank { ip }, ip.trim(), (port.toIntOrNull() ?: 0).toUShort()) }
                        Core.reload(); toast("Added"); name = ""; ip = ""; port = ""
                    }
                }, enabled = ip.isNotBlank() && port.isNotBlank())
            }
        }
        item { CrashLogRow() }
        item {
            Tile(title = "Event log") {
                Field(filter, { filter = it }, "Filter by event name")
            }
        }
        items(Core.devLog.filter { filter.isBlank() || filter in it.name }.take(200)) { e ->
            Column(Modifier.fillMaxWidth().padding(horizontal = 4.dp)) {
                Row {
                    Text(e.name, style = MaterialTheme.typography.labelMedium, color = T.text, modifier = Modifier.weight(1f))
                    Muted(timeAgo(e.ts))
                }
                Text(e.payload.take(400), style = MaterialTheme.typography.bodySmall.copy(fontFamily = FontFamily.Monospace), color = T.text2)
            }
        }
    }
}

@Composable
private fun Kv(k: String, v: String) {
    Row {
        Text(k, Modifier.width(96.dp), style = MaterialTheme.typography.bodySmall, color = T.text3)
        Text(v, style = MaterialTheme.typography.bodySmall, color = T.text, fontFamily = FontFamily.Monospace)
    }
}
