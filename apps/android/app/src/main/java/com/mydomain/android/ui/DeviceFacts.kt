package com.mydomain.android.ui

import kotlinx.coroutines.launch
import com.mydomain.android.core.DeviceLabels
import com.mydomain.android.core.friendlyError
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.remember
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.runtime.getValue
import androidx.compose.material3.pulltorefresh.rememberPullToRefreshState
import androidx.compose.material3.pulltorefresh.PullToRefreshDefaults
import androidx.compose.material3.pulltorefresh.PullToRefreshBox
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.ui.platform.LocalDensity
import android.text.format.Formatter
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.pluralStringResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.mydomain.android.R
import com.mydomain.android.core.Core
import com.mydomain.android.core.DeviceKinds
import com.mydomain.android.core.LinkState
import com.mydomain.android.core.LinksMap
import com.mydomain.android.core.linkLabel
import com.mydomain.android.core.linkStateOf
import com.mydomain.android.core.timeAgo

// What the redesigned screens say about a device, from what it reported
// only (its kind, its status, the links map): nothing made up. A fact it
// never sent is left out.

/** Its operating system's name ("Android", "macOS"...), if it said. */
fun osName(node: String): String? {
    val os = DeviceKinds.kinds[node]?.os?.takeIf { it.isNotEmpty() }
        ?: Core.deviceStatus[node]?.platform?.takeIf { it == "android" }
    return when (os) {
        "android" -> "Android"
        "macos" -> "macOS"
        "windows" -> "Windows"
        "linux" -> "Linux"
        "ios" -> "iOS"
        "chromeos" -> "ChromeOS"
        else -> null
    }
}

/** Connected now (directly or through the internet). */
fun isOnline(node: String): Boolean = Core.links.containsKey(node)

/** Which direct link it is ("Local network", "Bluetooth"...), from the
 *  links map; null unless directly linked (the status already says
 *  "through the internet") or when not known. */
@Composable
fun viaText(node: String): String? {
    if (linkStateOf(node) != LinkState.Direct) return null
    val how = LinksMap.info?.devices?.firstOrNull { it.nodeId == node }?.link?.how ?: return null
    return stringResource(howRes(how))
}

/** "Last seen 2 h ago", from the links map, while it's offline. */
@Composable
fun lastSeenText(node: String): String? {
    if (isOnline(node)) return null
    val secs = LinksMap.info?.devices?.firstOrNull { it.nodeId == node }?.lastSeenAgoS ?: return null
    return stringResource(R.string.dev_last_seen, timeAgo(System.currentTimeMillis() - secs * 1000))
}

/** Keep the links map read while a screen that shows it is open. */
@Composable
fun FollowLinksMap() {
    LaunchedEffect(LinksMap.changes) { runCatching { LinksMap.load() } }
}

/** The status colour for [s] (StatusPill, rows). */
@Composable
fun stateColor(s: LinkState) = when (s) {
    LinkState.Direct -> T.success
    LinkState.Relay -> T.warning
    LinkState.Failed -> T.danger
    LinkState.Connecting -> T.accent
    LinkState.Offline -> T.text3
}

/** A battery level's colour: amber when low and not charging (an alert). */
@Composable
fun batteryHue(level: Int, charging: Boolean?): Hue = if (level <= 20 && charging != true) Hue.Amber else Hue.Green

/** The chips under a device's name: its OS, battery and network kind, each
 *  only when it said. */
@OptIn(ExperimentalLayoutApi::class)
@Composable
fun FactChips(node: String) {
    val st = Core.deviceStatus[node]
    val os = osName(node)
    val wifi = st?.wifiConnected == true
    if (os == null && st?.battery == null && !wifi) return
    FlowRow(horizontalArrangement = Arrangement.spacedBy(6.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
        os?.let { InfoChip(if (DeviceKinds.kinds[node]?.os == "android") R.drawable.ic_lucide_smartphone else R.drawable.ic_lucide_monitor, it, Hue.Cyan) }
        st?.battery?.let { b ->
            InfoChip(
                if (st.charging == true) R.drawable.ic_lucide_battery_charging else R.drawable.ic_lucide_battery,
                stringResource(R.string.dev_percent, b),
                batteryHue(b, st.charging),
            )
        }
        if (wifi) InfoChip(R.drawable.ic_lucide_wifi, stringResource(R.string.dev_wifi), Hue.Blue)
    }
}

/** The device art in a soft round (the device page's avatar). */
@Composable
fun ArtAvatar(node: String, size: Dp) {
    Box(
        Modifier.size(size).clip(CircleShape).background(T.surface2).clearAndSetSemantics {},
        contentAlignment = Alignment.Center,
    ) { DeviceArt(node, size * 0.62f, maxWidth = size * 0.78f) }
}

/** The colour you gave a device (its label), as a small dot; nothing when none. */
@Composable
fun LabelDot(node: String) {
    val c = labelColor(DeviceLabels.labels[node]?.color) ?: return
    Box(Modifier.padding(end = 6.dp).size(8.dp).background(c, CircleShape))
}

/** A device as a row (Devices, Home's "Your devices"): its art, its colour
 *  and name, battery and a short status, and a chevron; opens its page. */
@Composable
fun DeviceListRow(node: String, onOpen: () -> Unit) {
    val name = Core.peerName(node)
    val st = Core.deviceStatus[node]
    val unread = Attention.newFrom(node)
    val s = linkStateOf(node)
    val online = s == LinkState.Direct || s == LinkState.Relay
    Row(
        Modifier.fillMaxWidth().heightIn(min = 72.dp).clip(RoundedCornerShape(16.dp))
            .clickable(role = Role.Button, onClick = onOpen).padding(horizontal = 12.dp, vertical = 8.dp)
            .semantics(mergeDescendants = true) {},
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Box(Modifier.width(64.dp), contentAlignment = Alignment.Center) { DeviceArt(node, 48.dp, maxWidth = 64.dp) }
        Spacer(Modifier.width(12.dp))
        Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(3.dp)) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                LabelDot(node)
                Text(
                    name, style = MaterialTheme.typography.titleSmall, color = T.text,
                    fontWeight = FontWeight.SemiBold, maxLines = 1, overflow = TextOverflow.Ellipsis,
                )
            }
            Row(verticalAlignment = Alignment.CenterVertically) {
                // The battery first, so a long status never pushes it out.
                st?.battery?.let { b ->
                    LIcon(if (st.charging == true) R.drawable.ic_lucide_battery_charging else R.drawable.ic_lucide_battery, hue(batteryHue(b, st.charging)), 14.dp)
                    Spacer(Modifier.width(3.dp))
                    Text(stringResource(R.string.dev_percent, b), style = MaterialTheme.typography.labelMedium, color = T.text2)
                    Spacer(Modifier.width(10.dp))
                }
                Box(Modifier.size(8.dp).background(stateColor(s), CircleShape))
                Spacer(Modifier.width(6.dp))
                val words = when (s) {
                    LinkState.Connecting, LinkState.Failed -> linkLabel(s)
                    else -> listOfNotNull(stringResource(if (online) R.string.dev_online else R.string.dev_offline), viaText(node)).joinToString(" · ")
                }
                Text(
                    words, Modifier.weight(1f, fill = false), style = MaterialTheme.typography.labelMedium,
                    color = if (online) stateColor(s) else T.text2, maxLines = 1, overflow = TextOverflow.Ellipsis,
                )
            }
            lastSeenText(node)?.let { Text(it, style = MaterialTheme.typography.bodySmall, color = T.text3, maxLines = 1, overflow = TextOverflow.Ellipsis) }
        }
        if (unread > 0) {
            val unreadText = pluralStringResource(R.plurals.a11y_unread, unread, unread)
            Box(
                Modifier.padding(start = 8.dp).background(T.accent, CircleShape).padding(horizontal = 8.dp, vertical = 2.dp)
                    .clearAndSetSemantics { contentDescription = unreadText },
            ) { Text("$unread", color = T.onAccent, style = MaterialTheme.typography.labelMedium, fontWeight = FontWeight.ExtraBold) }
        }
        Spacer(Modifier.width(8.dp))
        Chevron()
    }
}

/** Battery, storage and memory as stat cards, each only when [node]
 *  reported it (Home, under the hero), two to a row with large text.
 *  Tapping opens its page. */
@OptIn(ExperimentalLayoutApi::class)
@Composable
fun StatRow(node: String, onOpen: () -> Unit) {
    val st = Core.deviceStatus[node] ?: return
    val ctx = LocalContext.current
    val size = { n: Long -> Formatter.formatShortFileSize(ctx, n) }
    val cards = buildList<@Composable (Modifier) -> Unit> {
        st.battery?.let { b ->
            add { m ->
                StatCard(
                    if (st.charging == true) R.drawable.ic_lucide_battery_charging else R.drawable.ic_lucide_battery,
                    batteryHue(b, st.charging), stringResource(R.string.dev_percent, b), stringResource(R.string.info_battery),
                    if (st.charging == true) stringResource(R.string.dev_charging) else null, m, onOpen,
                )
            }
        }
        val disk = st.storageTotal?.takeIf { it > 0 }
        if (disk != null && st.storageFree != null) {
            val used = ((disk - st.storageFree.coerceIn(0, disk)) * 100 / disk).toInt()
            add { m ->
                StatCard(
                    R.drawable.ic_lucide_hard_drive, Hues.tools, stringResource(R.string.dev_percent, used),
                    stringResource(R.string.info_storage), stringResource(R.string.dev_free, size(st.storageFree)), m, onOpen,
                )
            }
        }
        val ram = st.ramTotal?.takeIf { it > 0 }
        if (ram != null && st.ramFree != null) {
            val used = ((ram - st.ramFree.coerceIn(0, ram)) * 100 / ram).toInt()
            add { m ->
                StatCard(
                    R.drawable.ic_lucide_memory_stick, Hues.tools, stringResource(R.string.dev_percent, used),
                    stringResource(R.string.info_memory), stringResource(R.string.dev_of, size(ram - st.ramFree.coerceIn(0, ram)), size(ram)), m, onOpen,
                )
            }
        }
    }
    if (cards.isEmpty()) return
    val perRow = if (LocalDensity.current.fontScale > 1.15f) 2 else 3
    FlowRow(
        Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(8.dp),
        verticalArrangement = Arrangement.spacedBy(8.dp), maxItemsInEachRow = perRow,
    ) {
        cards.forEach { c -> c(Modifier.weight(1f)) }
    }
}

/** Pull down to read the device list again (Home and Devices). */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun RefreshDevices(content: @Composable () -> Unit) {
    val toast = LocalToast.current
    val scope = rememberCoroutineScope()
    val pull = rememberPullToRefreshState()
    var refreshing by remember { mutableStateOf(false) }
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
    ) { content() }
}

/** Padding for the main screens' lists. */
val ScreenPadding = PaddingValues(start = 16.dp, end = 16.dp, top = 8.dp, bottom = 24.dp)
