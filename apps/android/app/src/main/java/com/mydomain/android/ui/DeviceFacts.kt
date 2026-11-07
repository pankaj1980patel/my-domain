package com.mydomain.android.ui

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

/** How it's linked now ("Local network", "Relay server"...), from the
 *  links map; null when not linked or not known. */
@Composable
fun viaText(node: String): String? {
    if (!isOnline(node)) return null
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

/** The status line's colour for [s]. */
@Composable
fun stateColor(s: LinkState) = when (s) {
    LinkState.Direct -> T.success
    LinkState.Relay -> T.warning
    LinkState.Failed -> T.danger
    LinkState.Connecting -> T.accent
    LinkState.Offline -> T.text3
}

/** A dot and the plain-words status, small. */
@Composable
fun StatusLine(node: String) {
    val s = linkStateOf(node)
    val c = stateColor(s)
    Row(verticalAlignment = Alignment.CenterVertically) {
        Box(Modifier.size(8.dp).background(c, CircleShape))
        Spacer(Modifier.width(6.dp))
        Text(
            linkLabel(s), style = MaterialTheme.typography.labelMedium,
            color = if (s == LinkState.Offline || s == LinkState.Connecting) T.text2 else c,
            maxLines = 1, overflow = TextOverflow.Ellipsis,
        )
    }
}

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
                if (b <= 20 && st.charging != true) Hue.Red else Hue.Green,
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

/** A device as a row (Devices, Home's "Your devices"): its art, name,
 *  status, battery when it said, and a chevron; opens its page. */
@Composable
fun DeviceListRow(node: String, onOpen: () -> Unit) {
    val name = Core.peerName(node)
    val st = Core.deviceStatus[node]
    val unread = Core.messages.count { it.node == node && it.incoming }
    Row(
        Modifier.fillMaxWidth().heightIn(min = 72.dp).clip(RoundedCornerShape(16.dp))
            .clickable(role = Role.Button, onClick = onOpen).padding(horizontal = 12.dp, vertical = 8.dp)
            .semantics(mergeDescendants = true) {},
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Box(Modifier.width(64.dp), contentAlignment = Alignment.Center) { DeviceArt(node, 48.dp, maxWidth = 64.dp) }
        Spacer(Modifier.width(12.dp))
        Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(3.dp)) {
            Text(
                name, Modifier.semantics { heading() }, style = MaterialTheme.typography.titleSmall, color = T.text,
                fontWeight = FontWeight.SemiBold, maxLines = 1, overflow = TextOverflow.Ellipsis,
            )
            Row(verticalAlignment = Alignment.CenterVertically) {
                StatusLine(node)
                st?.battery?.let { b ->
                    Spacer(Modifier.width(10.dp))
                    LIcon(if (st.charging == true) R.drawable.ic_lucide_battery_charging else R.drawable.ic_lucide_battery, hue(Hue.Green), 14.dp)
                    Spacer(Modifier.width(3.dp))
                    Text(stringResource(R.string.dev_percent, b), style = MaterialTheme.typography.labelMedium, color = T.text2)
                }
            }
            lastSeenText(node)?.let { Text(it, style = MaterialTheme.typography.labelSmall, color = T.text3) }
        }
        if (unread > 0) {
            val unreadText = pluralStringResource(R.plurals.a11y_unread, unread, unread)
            Box(
                Modifier.padding(start = 8.dp).background(T.accent, CircleShape).padding(horizontal = 8.dp, vertical = 2.dp)
                    .clearAndSetSemantics { contentDescription = unreadText },
            ) { Text("$unread", color = T.onAccent, fontSize = 11.sp, fontWeight = FontWeight.ExtraBold) }
        }
        Spacer(Modifier.width(8.dp))
        Chevron()
    }
}

/** Battery, storage and memory as stat cards, each only when [node]
 *  reported it (Home, under the hero). Tapping opens its page. */
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
                    Hue.Green, stringResource(R.string.dev_percent, b), stringResource(R.string.info_battery),
                    if (st.charging == true) stringResource(R.string.dev_charging) else null, m, onOpen,
                )
            }
        }
        val disk = st.storageTotal?.takeIf { it > 0 }
        if (disk != null && st.storageFree != null) {
            val used = ((disk - st.storageFree.coerceIn(0, disk)) * 100 / disk).toInt()
            add { m ->
                StatCard(
                    R.drawable.ic_lucide_hard_drive, Hue.Violet, stringResource(R.string.dev_percent, used),
                    stringResource(R.string.info_storage), stringResource(R.string.dev_free, size(st.storageFree)), m, onOpen,
                )
            }
        }
        val ram = st.ramTotal?.takeIf { it > 0 }
        if (ram != null && st.ramFree != null) {
            val used = ((ram - st.ramFree.coerceIn(0, ram)) * 100 / ram).toInt()
            add { m ->
                StatCard(
                    R.drawable.ic_lucide_memory_stick, Hue.Blue, stringResource(R.string.dev_percent, used),
                    stringResource(R.string.info_memory), stringResource(R.string.dev_of, size(ram - st.ramFree.coerceIn(0, ram)), size(ram)), m, onOpen,
                )
            }
        }
    }
    if (cards.isEmpty()) return
    Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
        cards.forEach { c -> c(Modifier.weight(1f)) }
    }
}

/** Padding for the main screens' lists. */
val ScreenPadding = PaddingValues(start = 16.dp, end = 16.dp, top = 8.dp, bottom = 24.dp)
