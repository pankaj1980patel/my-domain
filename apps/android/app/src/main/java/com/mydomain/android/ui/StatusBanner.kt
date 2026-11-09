package com.mydomain.android.ui

import android.content.Context
import android.net.ConnectivityManager
import android.net.Network
import android.net.NetworkCapabilities
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.onClick
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import com.mydomain.android.R
import com.mydomain.android.core.ConnBanner
import com.mydomain.android.core.Health

/** Whether the phone has a network with internet, kept up to date. */
@Composable
private fun rememberOnline(): Boolean {
    val ctx = LocalContext.current
    val cm = remember(ctx) { ctx.getSystemService(Context.CONNECTIVITY_SERVICE) as? ConnectivityManager }
    fun now(): Boolean = runCatching {
        cm?.getNetworkCapabilities(cm.activeNetwork)?.hasCapability(NetworkCapabilities.NET_CAPABILITY_INTERNET) == true
    }.getOrDefault(true)
    var online by remember { mutableStateOf(now()) }
    DisposableEffect(cm) {
        val watch = object : ConnectivityManager.NetworkCallback() {
            override fun onAvailable(network: Network) { online = now() }
            override fun onLost(network: Network) { online = now() }
            override fun onCapabilitiesChanged(network: Network, caps: NetworkCapabilities) { online = now() }
        }
        val ok = runCatching { cm?.registerDefaultNetworkCallback(watch) }.isSuccess
        onDispose { if (ok) runCatching { cm?.unregisterNetworkCallback(watch) } }
    }
    return online
}

/**
 * A slim line under the top bar while something's wrong with the
 * connection: "You’re offline", "Server unreachable" or "Reconnecting…",
 * from the phone's network and the health check's server line
 * ([ConnBanner]). Tap: Connections. Nothing at all while all is well.
 */
@Composable
fun ConnectionBanner(modifier: Modifier = Modifier, onOpen: () -> Unit) {
    val ctx = LocalContext.current
    val online = rememberOnline()
    var recovering by remember { mutableStateOf(false) }
    var wasOnline by remember { mutableStateOf(online) }
    // Back online: ask the server again now, "Reconnecting…" meanwhile.
    LaunchedEffect(online) {
        if (online && !wasOnline) {
            recovering = true
            try {
                Health.run(ctx, server = true)
            } finally {
                recovering = false
            }
        }
        wasOnline = online
    }
    val server = Health.checks.firstOrNull { it.id == "server" }?.reason
    val state = ConnBanner.state(online, server, recovering) ?: return
    val (icon, words) = when (state) {
        ConnBanner.State.OFFLINE -> R.drawable.ic_lucide_wifi to stringResource(R.string.banner_offline)
        ConnBanner.State.RECONNECTING -> R.drawable.ic_lucide_refresh_cw to stringResource(R.string.banner_reconnecting)
        ConnBanner.State.SERVER_UNREACHABLE -> R.drawable.ic_lucide_triangle_alert to stringResource(R.string.banner_server)
    }
    val openLabel = stringResource(R.string.banner_open)
    Row(
        modifier.fillMaxWidth().background(T.bar)
            .clickable(role = Role.Button, onClick = onOpen)
            .semantics(mergeDescendants = true) { onClick(openLabel) { onOpen(); true } }
            .announce()
            .heightIn(min = 48.dp).padding(horizontal = 16.dp, vertical = 6.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        LIcon(icon, T.warning, 18.dp)
        Spacer(Modifier.width(10.dp))
        Text(words, Modifier.weight(1f), style = MaterialTheme.typography.labelLarge, color = T.text)
        Text(openLabel, style = MaterialTheme.typography.labelMedium, color = T.accentFg)
        Spacer(Modifier.width(4.dp))
        Chevron()
    }
}
