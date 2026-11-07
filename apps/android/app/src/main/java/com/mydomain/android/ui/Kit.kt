package com.mydomain.android.ui

import androidx.annotation.DrawableRes
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.RowScope
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.selection.selectable
import androidx.compose.foundation.selection.selectableGroup
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Badge
import androidx.compose.material3.BadgedBox
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.mydomain.android.R

// The redesign's building blocks (the cosmic design boards): soft filled
// cards with no bright outlines, round coloured icons, pill tabs and chips,
// grouped action cards, rows with a chevron. Icons are Lucide (the desktop's
// set) as vector drawables, res/drawable/ic_lucide_*.

/** Semantic icon colours (the desktop's --c-* tokens): one meaning each,
 *  darker on the light theme so they keep 3:1. */
enum class Hue { Blue, Violet, Green, Amber, Red, Cyan, Muted }

/** The colour of [h] in the current theme. */
@Composable
fun hue(h: Hue): Color {
    val light = T.isLight
    return when (h) {
        Hue.Blue -> if (light) Color(0xFF1D4ED8) else Color(0xFF60A5FA)
        Hue.Violet -> if (light) Color(0xFF6D28D9) else Color(0xFFA78BFA)
        Hue.Green -> if (light) Color(0xFF047857) else Color(0xFF34D399)
        Hue.Amber -> if (light) Color(0xFF9A4508) else Color(0xFFFFC857)
        Hue.Red -> if (light) Color(0xFFBE123C) else Color(0xFFFB7185)
        Hue.Cyan -> if (light) Color(0xFF0369A1) else Color(0xFF22C7F2)
        Hue.Muted -> T.text2
    }
}

/** What each kind of thing is coloured: files blue, messages violet, calls
 *  green, alerts amber, danger red; tools cyan. */
object Hues {
    val files = Hue.Blue
    val messages = Hue.Violet
    val calls = Hue.Green
    val alerts = Hue.Amber
    val danger = Hue.Red
    val tools = Hue.Cyan
}

/** A Lucide icon ([res]: R.drawable.ic_lucide_*), tinted. Decorative unless
 *  [description] is given. */
@Composable
fun LIcon(@DrawableRes res: Int, tint: Color = T.text, size: Dp = 22.dp, description: String? = null, modifier: Modifier = Modifier) {
    Icon(painterResource(res), description, modifier.size(size), tint = tint)
}

/** A soft filled card: no outline, told apart by its fill (quiet borders). */
@Composable
fun SoftCard(
    modifier: Modifier = Modifier,
    onClick: (() -> Unit)? = null,
    padding: PaddingValues = PaddingValues(16.dp),
    color: Color = T.surface,
    content: @Composable ColumnScope.() -> Unit,
) {
    val shape = RoundedCornerShape(20.dp)
    val body: @Composable () -> Unit = {
        Column(Modifier.padding(padding), verticalArrangement = Arrangement.spacedBy(10.dp), content = content)
    }
    if (onClick != null) {
        Surface(onClick = onClick, modifier = modifier.fillMaxWidth(), shape = shape, color = color, content = body)
    } else {
        Surface(modifier = modifier.fillMaxWidth(), shape = shape, color = color, content = body)
    }
}

/** A round icon on a soft wash of its colour. */
@Composable
fun IconBubble(@DrawableRes icon: Int, h: Hue, size: Dp = 40.dp) {
    val c = hue(h)
    Box(
        Modifier.size(size).clip(CircleShape).background(c.copy(alpha = if (T.isLight) 0.12f else 0.18f)).clearAndSetSemantics {},
        contentAlignment = Alignment.Center,
    ) { LIcon(icon, c, size * 0.5f) }
}

/** A round coloured action with its label under it (Home's quick actions). */
@Composable
fun QuickAction(@DrawableRes icon: Int, h: Hue, label: String, onClick: () -> Unit, modifier: Modifier = Modifier, enabled: Boolean = true) {
    Column(
        modifier.alpha(if (enabled) 1f else 0.45f).clip(RoundedCornerShape(16.dp))
            .clickable(enabled = enabled, role = Role.Button, onClick = onClick).padding(vertical = 6.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.spacedBy(6.dp),
    ) {
        IconBubble(icon, h, 52.dp)
        Text(
            label, style = MaterialTheme.typography.labelMedium, color = T.text, textAlign = TextAlign.Center,
            maxLines = 2, overflow = TextOverflow.Ellipsis,
        )
    }
}

/** One choice of a [Pills] row: [value], its words, an optional icon. */
data class Pill<V>(val value: V, val label: String, @DrawableRes val icon: Int? = null)

/** Single-choice pills in a row that scrolls sideways (tabs and filter
 *  chips): the chosen one filled with the accent wash. */
@Composable
fun <V> Pills(options: List<Pill<V>>, selected: V, onSelect: (V) -> Unit, modifier: Modifier = Modifier) {
    Row(
        modifier.fillMaxWidth().horizontalScroll(rememberScrollState()).selectableGroup(),
        horizontalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        options.forEach { p ->
            val on = p.value == selected
            Row(
                Modifier.heightIn(min = 40.dp).clip(CircleShape)
                    .background(if (on) T.accentBg else T.surface)
                    .selectable(selected = on, role = Role.Tab, onClick = { onSelect(p.value) })
                    .padding(horizontal = 14.dp, vertical = 8.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                p.icon?.let {
                    LIcon(it, if (on) T.accentFg else T.text2, 18.dp)
                    Spacer(Modifier.width(6.dp))
                }
                Text(
                    p.label, style = MaterialTheme.typography.labelLarge,
                    color = if (on) T.accentFg else T.text2, fontWeight = if (on) FontWeight.Bold else FontWeight.Medium,
                )
            }
        }
    }
}

/** A small rounded chip: an icon and a few words (OS, battery, network). */
@Composable
fun InfoChip(@DrawableRes icon: Int?, text: String, h: Hue = Hue.Muted) {
    Row(
        Modifier.clip(CircleShape).background(T.surface2).padding(horizontal = 10.dp, vertical = 5.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        icon?.let {
            LIcon(it, hue(h), 14.dp)
            Spacer(Modifier.width(5.dp))
        }
        Text(text, style = MaterialTheme.typography.labelMedium, color = T.text)
    }
}

/** A chevron at a row's end (decorative). */
@Composable
fun Chevron() {
    LIcon(R.drawable.ic_lucide_chevron_right, T.text3, 18.dp, modifier = Modifier.clearAndSetSemantics {})
}

/** A row: round icon, title and an optional line, then [trailing] (a
 *  chevron by default). One TalkBack stop when tappable. */
@Composable
fun NavRow(
    @DrawableRes icon: Int,
    h: Hue,
    title: String,
    subtitle: String? = null,
    onClick: (() -> Unit)? = null,
    modifier: Modifier = Modifier,
    trailing: (@Composable RowScope.() -> Unit)? = { Chevron() },
) {
    var m = modifier.fillMaxWidth().heightIn(min = 56.dp).clip(RoundedCornerShape(16.dp))
    if (onClick != null) m = m.clickable(role = Role.Button, onClick = onClick)
    Row(m.padding(horizontal = 12.dp, vertical = 10.dp), verticalAlignment = Alignment.CenterVertically) {
        IconBubble(icon, h, 40.dp)
        Spacer(Modifier.width(12.dp))
        Column(Modifier.weight(1f)) {
            Text(title, style = MaterialTheme.typography.titleSmall, color = T.text, maxLines = 2, overflow = TextOverflow.Ellipsis)
            if (!subtitle.isNullOrEmpty()) {
                Text(subtitle, style = MaterialTheme.typography.bodySmall, color = T.text2, maxLines = 2, overflow = TextOverflow.Ellipsis)
            }
        }
        if (trailing != null) {
            Spacer(Modifier.width(8.dp))
            trailing()
        }
    }
}

/** A card of [NavRow]s with a little space between. */
@Composable
fun RowGroup(title: String? = null, content: @Composable ColumnScope.() -> Unit) {
    Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
        if (title != null) SectionTitle(title)
        SoftCard(padding = PaddingValues(4.dp)) {
            Column(verticalArrangement = Arrangement.spacedBy(0.dp), content = content)
        }
    }
}

/** A section's heading, with an optional link at its end ("View all"). */
@Composable
fun SectionTitle(title: String, action: String? = null, onAction: (() -> Unit)? = null) {
    Row(Modifier.fillMaxWidth().padding(top = 6.dp, start = 4.dp), verticalAlignment = Alignment.CenterVertically) {
        Text(
            title, Modifier.weight(1f).semantics { heading() },
            style = MaterialTheme.typography.titleMedium, color = T.text, fontWeight = FontWeight.Bold,
        )
        if (action != null && onAction != null) {
            Text(
                action,
                Modifier.heightIn(min = 40.dp).clip(CircleShape).clickable(role = Role.Button, onClick = onAction)
                    .padding(horizontal = 10.dp, vertical = 10.dp),
                style = MaterialTheme.typography.labelLarge, color = T.accentFg,
            )
        }
    }
}

/** One action of a [GroupCard]: an icon over its words. */
data class TileAction(
    @DrawableRes val icon: Int,
    val label: String,
    val h: Hue,
    val enabled: Boolean = true,
    val onClick: () -> Unit,
)

/** A grouped card (the device page): a header (icon, title, line,
 *  chevron) over a row of action tiles, three to a row. */
@Composable
fun GroupCard(
    @DrawableRes icon: Int,
    h: Hue,
    title: String,
    subtitle: String?,
    onHeader: (() -> Unit)?,
    actions: List<TileAction>,
    modifier: Modifier = Modifier,
) {
    SoftCard(modifier, padding = PaddingValues(start = 6.dp, end = 6.dp, top = 6.dp, bottom = 12.dp)) {
        NavRow(icon, h, title, subtitle, onHeader, trailing = if (onHeader != null) ({ Chevron() }) else null)
        actions.chunked(3).forEach { row ->
            Row(Modifier.fillMaxWidth().padding(horizontal = 6.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                row.forEach { a -> ActionTile(a, Modifier.weight(1f)) }
                repeat(3 - row.size) { Spacer(Modifier.weight(1f)) }
            }
        }
    }
}

@Composable
fun ActionTile(a: TileAction, modifier: Modifier = Modifier) {
    Column(
        modifier.heightIn(min = 76.dp).clip(RoundedCornerShape(14.dp)).background(T.surface2)
            .alpha(if (a.enabled) 1f else 0.45f)
            .clickable(enabled = a.enabled, role = Role.Button, onClick = a.onClick)
            .padding(horizontal = 10.dp, vertical = 12.dp),
        verticalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        LIcon(a.icon, hue(a.h), 22.dp)
        Text(a.label, style = MaterialTheme.typography.labelLarge, color = T.text, maxLines = 2, overflow = TextOverflow.Ellipsis)
    }
}

/** A small stat: icon, big value, label and a muted line (Home). */
@Composable
fun StatCard(@DrawableRes icon: Int, h: Hue, value: String, label: String, sub: String?, modifier: Modifier = Modifier, onClick: (() -> Unit)? = null) {
    SoftCard(modifier.semantics(mergeDescendants = true) {}, onClick = onClick, padding = PaddingValues(12.dp)) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            LIcon(icon, hue(h), 20.dp)
            Spacer(Modifier.width(6.dp))
            Text(value, style = MaterialTheme.typography.titleLarge, color = T.text, fontWeight = FontWeight.Bold, maxLines = 1)
        }
        Column {
            Text(label, style = MaterialTheme.typography.labelMedium, color = T.text2, maxLines = 1, overflow = TextOverflow.Ellipsis)
            if (sub != null) Text(sub, style = MaterialTheme.typography.labelSmall, color = T.text3, maxLines = 1, overflow = TextOverflow.Ellipsis)
        }
    }
}

/** The bar at the top of the main tabs: the logo and wordmark, the bell
 *  (with how many need a look) and Settings (a dot when the health check
 *  found something red). */
@Composable
fun TopBar(bellCount: Int, onBell: () -> Unit, settingsDot: Boolean, onSettings: () -> Unit) {
    Row(
        Modifier.fillMaxWidth().statusBarsPadding().padding(start = 16.dp, end = 4.dp, top = 6.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Image(painterResource(logoRes()), null, Modifier.size(40.dp))
        Spacer(Modifier.width(10.dp))
        Column(Modifier.weight(1f)) {
            Wordmark(fontSize = 20)
            Tagline()
        }
        val bellLabel = if (bellCount > 0) {
            androidx.compose.ui.res.pluralStringResource(R.plurals.top_bell_count, bellCount, bellCount)
        } else {
            stringResource(R.string.top_bell)
        }
        IconButton(onClick = onBell, modifier = Modifier.semantics { contentDescription = bellLabel }) {
            BadgedBox(badge = {
                if (bellCount > 0) {
                    Badge(containerColor = T.danger, contentColor = Color.White) {
                        Text(if (bellCount > 99) "99+" else "$bellCount", fontSize = 10.sp)
                    }
                }
            }) { LIcon(R.drawable.ic_lucide_bell, T.text, 24.dp) }
        }
        val settingsLabel = stringResource(R.string.tab_settings)
        IconButton(onClick = onSettings, modifier = Modifier.semantics { contentDescription = settingsLabel }) {
            BadgedBox(badge = { if (settingsDot) Badge(containerColor = T.danger) }) {
                LIcon(R.drawable.ic_lucide_settings, T.text, 24.dp)
            }
        }
    }
}

/** A pushed screen's bar: Back, the title in the middle, and [actions]. */
@Composable
fun BackBar(title: String, onBack: () -> Unit, actions: (@Composable RowScope.() -> Unit)? = null) {
    Row(Modifier.fillMaxWidth().heightIn(min = 56.dp).padding(horizontal = 4.dp), verticalAlignment = Alignment.CenterVertically) {
        IconButton(onClick = onBack) { LIcon(R.drawable.ic_lucide_arrow_left, T.text, 24.dp, stringResource(R.string.back)) }
        Text(
            title, Modifier.weight(1f).semantics { heading() }, style = MaterialTheme.typography.titleLarge,
            color = T.text, fontWeight = FontWeight.Bold, maxLines = 1, overflow = TextOverflow.Ellipsis,
        )
        if (actions != null) Row(verticalAlignment = Alignment.CenterVertically, content = actions)
    }
}

/** A coloured dot and words: "Online" / "Offline" (a pill). */
@Composable
fun OnlinePill(online: Boolean) {
    val c = if (online) T.success else T.text3
    Row(
        Modifier.clip(CircleShape).background(c.copy(alpha = 0.16f)).padding(horizontal = 12.dp, vertical = 6.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Box(Modifier.size(8.dp).background(c, CircleShape))
        Spacer(Modifier.width(6.dp))
        Text(
            stringResource(if (online) R.string.dev_online else R.string.dev_offline),
            style = MaterialTheme.typography.labelLarge, color = if (online) T.success else T.text2,
        )
    }
}

/** A limit for wide screens: content stays readable on a tablet. */
fun Modifier.readable(): Modifier = this.widthIn(max = 720.dp)
