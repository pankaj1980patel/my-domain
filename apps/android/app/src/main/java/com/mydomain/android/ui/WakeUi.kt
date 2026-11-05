package com.mydomain.android.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.size
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import com.mydomain.android.R
import com.mydomain.android.core.Core
import com.mydomain.android.core.LinkState
import com.mydomain.android.core.friendlyError
import com.mydomain.android.core.linkStateOf
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch

/** How long "Waking…" waits for the computer to connect. */
private const val WAKE_WAIT_MS = 60_000L

/** How often it tries to connect meanwhile. */
private const val WAKE_RETRY_MS = 10_000L

/**
 * "Wake it" on a computer's page while it's offline and said it may be
 * woken (Wake-on-LAN, see the core's features::wake): sends the magic
 * packet from here and from your other devices on its network, then shows
 * a spinner, trying to connect every 10 s, until it connects or a minute
 * passes.
 */
@Composable
fun WakeIt(node: String) {
    val toast = LocalToast.current
    val ctx = LocalContext.current
    val scope = rememberCoroutineScope()
    val s = linkStateOf(node)
    // Learnt from its status (kept across restarts): asked again when one comes.
    val status = Core.deviceStatus[node]
    val can = remember(node, status) { Core.canWake(node) }
    var waking by remember(node) { mutableStateOf(false) }
    val awake = s == LinkState.Direct || s == LinkState.Relay
    val name = Core.peerName(node)
    val awakeText = stringResource(R.string.wake_awake, name)
    val noAnswer = stringResource(R.string.wake_no_answer, name)

    // It connected: awake.
    LaunchedEffect(waking, awake) {
        if (waking && awake) {
            waking = false
            toast(awakeText)
        }
    }
    // Meanwhile, try to connect now and then; give up after a minute.
    LaunchedEffect(node, waking) {
        if (!waking) return@LaunchedEffect
        val until = System.currentTimeMillis() + WAKE_WAIT_MS
        while (System.currentTimeMillis() < until) {
            delay(WAKE_RETRY_MS)
            if (linkStateOf(node) != LinkState.Connecting) runCatching { Core.connect(node) }
        }
        waking = false
        toast(noAnswer)
    }

    if (!can || !(waking || s == LinkState.Offline || s == LinkState.Failed)) return
    Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
        if (waking) {
            Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                CircularProgressIndicator(Modifier.size(16.dp), color = T.accent, strokeWidth = 2.dp)
                Muted(stringResource(R.string.wake_waking), Modifier.announce())
            }
        } else {
            GhostButton(stringResource(R.string.wake_it), {
                scope.launch {
                    try {
                        val r = Core.wake(node)
                        toast(
                            if (r.relays.isEmpty()) ctx.getString(R.string.wake_sent)
                            else ctx.getString(R.string.wake_sent_relayed, r.relays.joinToString(", ") { Core.peerName(it) }),
                        )
                        waking = true
                    } catch (e: Exception) {
                        toast(friendlyError(e))
                    }
                }
            }, small = true)
        }
        Muted(stringResource(R.string.wake_hint))
    }
}
