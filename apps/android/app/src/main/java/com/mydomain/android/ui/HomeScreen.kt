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
import androidx.compose.ui.res.pluralStringResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import com.mydomain.android.R
import com.mydomain.android.core.Core
import com.mydomain.android.core.friendlyError
import kotlinx.coroutines.launch

/** Search everything (at the top), then your devices as cards, connected
 *  ones first. Pull down to refresh the list. */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun HomeScreen(
    onOpen: (String) -> Unit,
    onSetup: () -> Unit = {},
    onSearch: () -> Unit = {},
    onBattery: () -> Unit = {},
    onTimers: () -> Unit = {},
) {
    val toast = LocalToast.current
    val scope = rememberCoroutineScope()
    val pull = rememberPullToRefreshState()
    var refreshing by remember { mutableStateOf(false) }
    val devices = Core.peers.sortedWith(
        compareBy({ if (Core.links.containsKey(it.nodeId)) 0 else 1 }, { Core.peerName(it.nodeId).lowercase() }),
    )
    val online = Core.peers.count { Core.links.containsKey(it.nodeId) }
    val updated = stringResource(R.string.home_list_updated)

    PullToRefreshBox(
        isRefreshing = refreshing,
        onRefresh = {
            refreshing = true
            scope.launch {
                try { Core.refreshDevices(); toast(updated) } catch (e: Exception) { toast(friendlyError(e)) }
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
                containerColor = T.bar,
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
                    stringResource(R.string.home_title),
                    if (Core.peers.isEmpty()) stringResource(R.string.home_subtitle_empty)
                    else pluralStringResource(R.plurals.home_subtitle_connected, Core.peers.size, online, Core.peers.size),
                )
            }
            // One search over everything kept on this phone (SearchScreen).
            item { SearchEntry(onSearch) }
            // Countdown timers shared with your other devices (TimersScreen).
            item { TimersEntry(onTimers) }
            // Once after sign-in: what this phone can turn on (SetupScreen).
            item { SetupHomeCard(onSetup) }
            // While heavy background work waits to save battery (SaveBatteryTile).
            item { SavingBatteryBanner(onBattery) }
            if (devices.isEmpty()) {
                item {
                    EmptyState(
                        stringResource(R.string.home_empty_title),
                        stringResource(R.string.home_empty_body),
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
