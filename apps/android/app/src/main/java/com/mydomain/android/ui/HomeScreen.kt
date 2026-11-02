package com.mydomain.android.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.pulltorefresh.PullToRefreshBox
import androidx.compose.material3.pulltorefresh.PullToRefreshDefaults
import androidx.compose.material3.pulltorefresh.rememberPullToRefreshState
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
import com.mydomain.android.core.friendlyError
import com.mydomain.android.core.niceName
import kotlinx.coroutines.launch

/** Your devices as cards, connected ones first. Pull down to refresh the list. */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun HomeScreen(onOpen: (String) -> Unit) {
    val toast = LocalToast.current
    val scope = rememberCoroutineScope()
    val pull = rememberPullToRefreshState()
    var refreshing by remember { mutableStateOf(false) }
    val devices = Core.peers.sortedWith(
        compareBy({ if (Core.links.containsKey(it.nodeId)) 0 else 1 }, { niceName(it.name).lowercase() }),
    )
    val online = Core.peers.count { Core.links.containsKey(it.nodeId) }

    PullToRefreshBox(
        isRefreshing = refreshing,
        onRefresh = {
            refreshing = true
            scope.launch {
                try { Core.refreshDevices(); toast("Device list updated") } catch (e: Exception) { toast(friendlyError(e)) }
                finally { refreshing = false }
            }
        },
        modifier = Modifier.fillMaxSize(),
        state = pull,
        indicator = {
            PullToRefreshDefaults.Indicator(
                state = pull,
                isRefreshing = refreshing,
                modifier = Modifier.align(Alignment.TopCenter),
                containerColor = SpaceBar,
                color = T.accent,
            )
        },
    ) {
        LazyColumn(
            Modifier.fillMaxSize(),
            contentPadding = PaddingValues(16.dp),
            verticalArrangement = Arrangement.spacedBy(10.dp),
        ) {
            item {
                PageHead(
                    "Your devices",
                    if (Core.peers.isEmpty()) "Devices signed in to your account appear here. Pull down to refresh."
                    else "$online of ${Core.peers.size} connected",
                )
            }
            if (devices.isEmpty()) {
                item {
                    EmptyState(
                        "No other devices yet",
                        "Install my-domain on your computer or another phone and sign in with the same account. It shows up here by itself.",
                    )
                }
            }
            items(devices, key = { it.nodeId }) { p ->
                DeviceCard(p, onOpen = { onOpen(p.nodeId) }, onConnect = {
                    scope.launch { try { Core.connect(p.nodeId) } catch (e: Exception) { toast(friendlyError(e)) } }
                })
            }
        }
    }
}
