package com.mydomain.android.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
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
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import com.mydomain.android.R
import com.mydomain.android.core.ActivityLog
import com.mydomain.android.core.ActivityQuery
import com.mydomain.android.core.ActivityRow
import com.mydomain.android.core.Core
import com.mydomain.android.core.Health
import com.mydomain.android.core.friendlyError
import java.util.Calendar
import kotlinx.coroutines.launch

/** Your devices, most connected first, then by name. */
fun sortedPeers(): List<String> = Core.peers.map { it.nodeId }.sortedWith(
    compareBy({ if (Core.links.containsKey(it)) 0 else 1 }, { Core.peerName(it).lowercase() }),
)

/** The device Home leads with: the one you used last (the newest activity
 *  with a device), else the first connected one, else the first. */
private fun heroPeer(): String? {
    val known = Core.peers.map { it.nodeId }.toSet()
    if (known.isEmpty()) return null
    val recent = runCatching { ActivityLog.page(ActivityQuery(limit = 20)).items.firstNotNullOfOrNull { a -> a.node?.takeIf { it in known } } }.getOrNull()
    return recent ?: sortedPeers().firstOrNull()
}

/**
 * Home: a greeting with how many devices are online; a hero card for the
 * device you used last (its art, name, how it's connected, its OS, battery
 * and network); round quick actions for it; the health check; its battery,
 * storage and memory when it reported them; your devices; recent activity.
 * Cards that need you (setup to finish, a passphrase change, saving
 * battery, the phone helper) come first. Pull down to refresh the list.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun HomeScreen(
    onOpen: (String) -> Unit,
    onOpenTab: (String, String) -> Unit = { n, _ -> onOpen(n) },
    onRename: (String) -> Unit = onOpen,
    onDevices: () -> Unit = {},
    onBattery: () -> Unit = {},
    onActivity: () -> Unit = {},
    onActivityRow: (ActivityRow) -> Unit = {},
    onHealth: () -> Unit = {},
    onOnboarding: () -> Unit = {},
) {
    val toast = LocalToast.current
    val scope = rememberCoroutineScope()
    val pull = rememberPullToRefreshState()
    var refreshing by remember { mutableStateOf(false) }
    val devices = sortedPeers()
    val online = devices.count { Core.links.containsKey(it) }
    val updated = stringResource(R.string.home_list_updated)
    val hero = remember(Core.peers, ActivityLog.version) { heroPeer() }
    val send = rememberFileSender { hero }
    FollowLinksMap()

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
            contentPadding = ScreenPadding,
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            item {
                Column(Modifier.padding(horizontal = 4.dp)) {
                    Text(
                        stringResource(greeting()), Modifier.semantics { heading() },
                        style = MaterialTheme.typography.headlineSmall, color = T.text, fontWeight = FontWeight.Bold,
                    )
                    Muted(
                        if (devices.isEmpty()) stringResource(R.string.home_subtitle_empty)
                        else pluralStringResource(R.plurals.home_online_of, devices.size, online, devices.size),
                    )
                }
            }
            // First-run setup not finished: carry on where it stopped.
            item { SetupResumeCard(onOnboarding) }
            // While heavy background work waits to save battery (SaveBatteryTile).
            item { SavingBatteryBanner(onBattery) }
            // A passphrase change made on another device missed here.
            item { KeyRotationPrompt() }
            // The phone helper is on: banking apps may refuse to work (Secure mode).
            item { SecureModeHomeCard() }
            if (hero == null) {
                item {
                    EmptyState(stringResource(R.string.home_empty_title), stringResource(R.string.home_empty_body))
                }
            } else {
                item(key = "hero") { HeroCard(hero, onOpen = { onOpen(hero) }, onRename = { onRename(hero) }) }
                item(key = "quick") {
                    val st = Core.deviceStatus[hero]
                    val ringing = stringResource(R.string.find_ringing, Core.peerName(hero))
                    val ringQueued = stringResource(R.string.find_ring_queued, Core.peerName(hero))
                    val asked = stringResource(R.string.clipboard_asked)
                    val screen = st != null && (st.screenView || "input" in st.actions)
                    SoftCard(padding = PaddingValues(horizontal = 6.dp, vertical = 10.dp)) {
                        Row(Modifier.fillMaxWidth()) {
                            QuickAction(R.drawable.ic_lucide_send, Hues.files, stringResource(R.string.home_q_send), { send.files() }, Modifier.weight(1f), !send.busy)
                            QuickAction(R.drawable.ic_lucide_clipboard, Hues.messages, stringResource(R.string.home_q_clipboard), {
                                scope.launch { try { Core.pullClipboard(hero); toast(asked) } catch (e: Exception) { toast(friendlyError(e)) } }
                            }, Modifier.weight(1f))
                            QuickAction(R.drawable.ic_lucide_monitor, Hues.tools, stringResource(R.string.home_q_screen), {
                                onOpenTab(hero, DeviceTab.Remote.key)
                            }, Modifier.weight(1f), screen)
                            QuickAction(R.drawable.ic_lucide_bell_ring, Hues.alerts, stringResource(R.string.home_q_ring), {
                                scope.launch {
                                    try { toast(if (Core.ring(hero) == "queued") ringQueued else ringing) } catch (e: Exception) { toast(friendlyError(e)) }
                                }
                            }, Modifier.weight(1f))
                            QuickAction(R.drawable.ic_lucide_layout_grid, Hue.Muted, stringResource(R.string.home_q_more), { onOpen(hero) }, Modifier.weight(1f))
                        }
                    }
                }
            }
            // The health check, good or not (HealthScreen).
            item(key = "health") { HealthCard(onHealth) }
            if (hero != null) item(key = "stats") { StatRow(hero) { onOpen(hero) } }
            if (devices.isNotEmpty()) {
                item {
                    SectionTitle(stringResource(R.string.home_title), stringResource(R.string.home_view_all), onDevices)
                }
                item {
                    SoftCard(padding = PaddingValues(4.dp)) {
                        devices.take(4).forEach { n -> DeviceListRow(n) { onOpen(n) } }
                    }
                }
                item { SectionTitle(stringResource(R.string.home_recent), stringResource(R.string.home_view_all), onActivity) }
                item { SoftCard(padding = PaddingValues(8.dp)) { RecentActivity(3, onActivityRow) } }
            }
        }
    }
}

/** "Good morning" / "Good afternoon" / "Good evening", by the clock. */
private fun greeting(): Int = when (Calendar.getInstance().get(Calendar.HOUR_OF_DAY)) {
    in 5..11 -> R.string.home_greet_morning
    in 12..17 -> R.string.home_greet_afternoon
    else -> R.string.home_greet_evening
}

/** The hero: the device's art, its name with a pencil, how it's connected,
 *  and chips for its OS, battery and network. Opens its page. */
@Composable
private fun HeroCard(node: String, onOpen: () -> Unit, onRename: () -> Unit) {
    val name = Core.peerName(node)
    SoftCard(onClick = onOpen) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Box(Modifier.width(124.dp), contentAlignment = Alignment.Center) { DeviceArt(node, 120.dp, maxWidth = 124.dp) }
            Spacer(Modifier.width(14.dp))
            Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(6.dp)) {
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Text(
                        name, Modifier.weight(1f, fill = false).semantics { heading() },
                        style = MaterialTheme.typography.titleLarge, color = T.text, fontWeight = FontWeight.Bold,
                        maxLines = 2, overflow = TextOverflow.Ellipsis,
                    )
                    IconButton(onClick = onRename, modifier = Modifier.size(40.dp)) {
                        LIcon(R.drawable.ic_lucide_pencil, T.text2, 18.dp, stringResource(R.string.dev_rename_named, name))
                    }
                }
                StatusPill(node)
                (viaText(node) ?: lastSeenText(node))?.let { Muted(it) }
                FactChips(node)
            }
        }
    }
}

/** The health check as a card: all good, worth a look, or needs fixing. */
@Composable
private fun HealthCard(onOpen: () -> Unit) {
    val checks = Health.checks
    val bad = Health.bad
    val warn = checks.count { it.level == "warn" }
    val (h, title, body) = when {
        checks.isEmpty() -> Triple(Hue.Muted, stringResource(R.string.home_health), stringResource(R.string.health_running))
        bad > 0 -> Triple(Hue.Red, stringResource(R.string.home_health_bad), stringResource(R.string.health_summary_bad, bad))
        warn > 0 -> Triple(Hue.Amber, stringResource(R.string.home_health_look), stringResource(R.string.health_summary_warn, warn))
        else -> Triple(Hue.Green, stringResource(R.string.home_health_good), stringResource(R.string.health_all_good))
    }
    SoftCard(onClick = onOpen) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            IconBubble(if (bad > 0 || warn > 0) R.drawable.ic_lucide_shield else R.drawable.ic_lucide_shield_check, h, 52.dp)
            Spacer(Modifier.width(14.dp))
            Column(Modifier.weight(1f)) {
                Text(stringResource(R.string.home_health), style = MaterialTheme.typography.labelLarge, color = T.text2)
                Text(title, style = MaterialTheme.typography.titleLarge, color = hue(h), fontWeight = FontWeight.Bold)
                Muted(body)
            }
            Chevron()
        }
    }
}
