package com.mydomain.android.ui

import androidx.annotation.DrawableRes
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.Image
import androidx.compose.foundation.ScrollState
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
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
import androidx.compose.material3.minimumInteractiveComponentSize
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableLongStateOf
import androidx.compose.runtime.mutableStateMapOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.drawWithContent
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.BlendMode
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.CompositingStrategy
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.layout.onGloballyPositioned
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.ui.layout.positionInParent
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.Hyphens
import androidx.compose.ui.text.style.LineBreak
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.mydomain.android.R
import com.mydomain.android.core.Core

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
 *  green, alerts amber, danger red; tools cyan. A colour means only that:
 *  navigation and settings rows with no such meaning are [Hue.Muted] (or
 *  [Hue.Cyan] when they are tools), and the same item keeps one colour
 *  everywhere. */
object Hues {
    val files = Hue.Blue
    val messages = Hue.Violet
    val calls = Hue.Green
    val alerts = Hue.Amber
    val danger = Hue.Red
    val tools = Hue.Cyan
    val plain = Hue.Muted
}

/** Messages that came in since you last opened Messages (this session):
 *  the top bar's bell and the unread badges on device rows. */
internal object Attention {
    var seenAt by mutableLongStateOf(0L)

    val newMessages: Int
        get() = Core.messages.count { it.incoming && it.ts > seenAt }

    fun newFrom(node: String): Int = Core.messages.count { it.node == node && it.incoming && it.ts > seenAt }
}

/** A Lucide icon ([res]: R.drawable.ic_lucide_*), tinted. Decorative unless
 *  [description] is given. */
@Composable
fun LIcon(@DrawableRes res: Int, tint: Color = T.text, size: Dp = 22.dp, description: String? = null, modifier: Modifier = Modifier) {
    Icon(painterResource(res), description, modifier.size(size), tint = tint)
}

/** A card's title (a heading). */
@Composable
fun CardTitle(title: String, modifier: Modifier = Modifier) {
    Text(title, modifier.semantics { heading() }, style = MaterialTheme.typography.titleMedium, color = T.text)
}

/** The one card: a soft fill, the theme's card corners, no outline (quiet
 *  borders), except a hairline on the light theme, where the fill alone is
 *  too close to the background. [title]: a heading on top. */
@Composable
fun SoftCard(
    modifier: Modifier = Modifier,
    onClick: (() -> Unit)? = null,
    padding: PaddingValues = PaddingValues(16.dp),
    color: Color = T.surface,
    title: String? = null,
    content: @Composable ColumnScope.() -> Unit,
) {
    val shape = MaterialTheme.shapes.medium
    val border = if (T.isLight) BorderStroke(1.dp, T.border) else null
    val body: @Composable () -> Unit = {
        Column(Modifier.padding(padding), verticalArrangement = Arrangement.spacedBy(10.dp)) {
            if (title != null) CardTitle(title)
            content()
        }
    }
    // Only a card you can tap is clickable: a disabled clickable Surface
    // would merge every line in it into one node read as "disabled".
    if (onClick != null) {
        Surface(onClick = onClick, modifier = modifier.fillMaxWidth(), shape = shape, color = color, border = border, content = body)
    } else {
        Surface(modifier = modifier.fillMaxWidth(), shape = shape, color = color, border = border, content = body)
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

/** Labels under icons: never broken mid-word (hyphenated where the
 *  language allows, else moved whole to the next line). */
@Composable
private fun tileLabelStyle(): TextStyle = MaterialTheme.typography.labelMedium.copy(hyphens = Hyphens.Auto, lineBreak = LineBreak.Heading)

/** One action: an icon over its words. Disabled for now ([enabled] false)
 *  with [reason] said under it and read out ("Offline", "Sending…");
 *  actions a device can't do at all are left out, not disabled. */
data class TileAction(
    @DrawableRes val icon: Int,
    val label: String,
    val h: Hue,
    val enabled: Boolean = true,
    val reason: String? = null,
    val onClick: () -> Unit,
)

private fun Modifier.disabledReason(a: TileAction): Modifier =
    if (!a.enabled && a.reason != null) semantics { stateDescription = a.reason } else this

/** A round coloured action with its label under it on one line (Home's
 *  quick actions). */
@Composable
fun QuickAction(a: TileAction, modifier: Modifier = Modifier) {
    Column(
        modifier.clip(RoundedCornerShape(16.dp))
            .clickable(enabled = a.enabled, role = Role.Button, onClick = a.onClick)
            .disabledReason(a).padding(vertical = 6.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.spacedBy(6.dp),
    ) {
        Box(Modifier.alpha(if (a.enabled) 1f else 0.45f)) { IconBubble(a.icon, a.h, 52.dp) }
        Text(
            a.label, style = tileLabelStyle(), color = if (a.enabled) T.text else T.text2, textAlign = TextAlign.Center,
            maxLines = 1, overflow = TextOverflow.Ellipsis,
        )
        if (!a.enabled && a.reason != null) {
            Text(a.reason, style = MaterialTheme.typography.bodySmall, color = T.text3, maxLines = 1, overflow = TextOverflow.Ellipsis)
        }
    }
}

/** One choice of a [Pills] row: [value], its words, an optional icon. */
data class Pill<V>(val value: V, val label: String, @DrawableRes val icon: Int? = null)

/** Fade the ends of a sideways-scrolling row where there's more to see. */
fun Modifier.fadingEdges(scroll: ScrollState, edge: Dp = 24.dp): Modifier = this
    .graphicsLayer { compositingStrategy = CompositingStrategy.Offscreen }
    .drawWithContent {
        drawContent()
        val w = edge.toPx()
        if (scroll.value > 0) {
            drawRect(
                Brush.horizontalGradient(0f to Color.Transparent, 1f to Color.Black, startX = 0f, endX = w),
                size = Size(w, size.height), blendMode = BlendMode.DstIn,
            )
        }
        if (scroll.value < scroll.maxValue) {
            drawRect(
                Brush.horizontalGradient(0f to Color.Black, 1f to Color.Transparent, startX = size.width - w, endX = size.width),
                topLeft = Offset(size.width - w, 0f), size = Size(w, size.height), blendMode = BlendMode.DstIn,
            )
        }
    }

/** Single-choice pills in a row that scrolls sideways (tabs: [role]
 *  Tab; filters: RadioButton): the chosen one filled with the accent wash
 *  and scrolled into view; the ends fade where the row goes on. */
@Composable
fun <V> Pills(
    options: List<Pill<V>>,
    selected: V,
    onSelect: (V) -> Unit,
    modifier: Modifier = Modifier,
    role: Role = Role.Tab,
) {
    val scroll = rememberScrollState()
    var viewport by remember { mutableIntStateOf(0) }
    // Each pill's left edge and width in the row, by index.
    val spots = remember { mutableStateMapOf<Int, Pair<Int, Int>>() }
    val at = options.indexOfFirst { it.value == selected }
    val spot = spots[at]
    LaunchedEffect(at, spot, viewport) {
        val (x, w) = spot ?: return@LaunchedEffect
        if (viewport == 0) return@LaunchedEffect
        scroll.animateScrollTo((x - (viewport - w) / 2).coerceIn(0, scroll.maxValue))
    }
    Row(
        modifier.fillMaxWidth().onSizeChanged { viewport = it.width }.fadingEdges(scroll)
            .horizontalScroll(scroll).selectableGroup(),
        horizontalArrangement = Arrangement.spacedBy(8.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        options.forEachIndexed { i, p ->
            val on = p.value == selected
            Row(
                Modifier.onGloballyPositioned { spots[i] = it.positionInParent().x.toInt() to it.size.width }
                    .minimumInteractiveComponentSize().heightIn(min = 40.dp).clip(CircleShape)
                    .background(if (on) T.accentBg else T.surface)
                    .selectable(selected = on, role = role, onClick = { onSelect(p.value) })
                    .padding(horizontal = 14.dp, vertical = 8.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                p.icon?.let {
                    LIcon(it, if (on) T.accentFg else T.text2, 18.dp)
                    Spacer(Modifier.width(6.dp))
                }
                Text(
                    p.label, style = MaterialTheme.typography.labelLarge, maxLines = 1,
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
 *  chevron by default). One TalkBack stop when tappable; [state] is read
 *  out with it (a badge's meaning). */
@Composable
fun NavRow(
    @DrawableRes icon: Int,
    h: Hue,
    title: String,
    subtitle: String? = null,
    onClick: (() -> Unit)? = null,
    modifier: Modifier = Modifier,
    state: String? = null,
    trailing: (@Composable RowScope.() -> Unit)? = { Chevron() },
) {
    var m = modifier.fillMaxWidth().heightIn(min = 56.dp).clip(RoundedCornerShape(16.dp))
    if (onClick != null) m = m.clickable(role = Role.Button, onClick = onClick)
    if (state != null) m = m.semantics(mergeDescendants = true) { stateDescription = state }
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

/** A card of [NavRow]s, with an optional heading above it. */
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
fun SectionTitle(title: String, action: String? = null, onAction: (() -> Unit)? = null, modifier: Modifier = Modifier) {
    Row(modifier.fillMaxWidth().padding(top = 6.dp, start = 4.dp), verticalAlignment = Alignment.CenterVertically) {
        Text(
            title, Modifier.weight(1f).semantics { heading() },
            style = MaterialTheme.typography.titleMedium, color = T.text, fontWeight = FontWeight.Bold,
        )
        if (action != null && onAction != null) {
            Text(
                action,
                Modifier.heightIn(min = 48.dp).clip(CircleShape).clickable(role = Role.Button, onClick = onAction)
                    .padding(horizontal = 10.dp, vertical = 14.dp),
                style = MaterialTheme.typography.labelLarge, color = T.accentFg,
            )
        }
    }
}

/** A grouped card (the device page): a header ([leading] or a round icon,
 *  title, line, chevron) over its action tiles, three to a row, or two on a
 *  narrow screen or with large text (so no label breaks mid-word). */
@Composable
fun GroupCard(
    @DrawableRes icon: Int,
    h: Hue,
    title: String,
    subtitle: String?,
    onHeader: (() -> Unit)?,
    actions: List<TileAction>,
    modifier: Modifier = Modifier,
    leading: (@Composable () -> Unit)? = null,
) {
    SoftCard(modifier, padding = PaddingValues(start = 6.dp, end = 6.dp, top = 6.dp, bottom = 12.dp)) {
        if (leading != null) {
            var m = Modifier.fillMaxWidth().heightIn(min = 56.dp).clip(RoundedCornerShape(16.dp))
            if (onHeader != null) m = m.clickable(role = Role.Button, onClick = onHeader)
            Row(m.padding(horizontal = 12.dp, vertical = 8.dp), verticalAlignment = Alignment.CenterVertically) {
                leading()
                Spacer(Modifier.width(12.dp))
                Column(Modifier.weight(1f)) {
                    Text(title, style = MaterialTheme.typography.titleSmall, color = T.text, maxLines = 2, overflow = TextOverflow.Ellipsis)
                    if (!subtitle.isNullOrEmpty()) Text(subtitle, style = MaterialTheme.typography.bodySmall, color = T.text2)
                }
                if (onHeader != null) Chevron()
            }
        } else {
            NavRow(icon, h, title, subtitle, onHeader, trailing = if (onHeader != null) ({ Chevron() }) else null)
        }
        if (actions.isNotEmpty()) {
            BoxWithConstraints(Modifier.fillMaxWidth().padding(horizontal = 6.dp)) {
                val perRow = if (maxWidth < 340.dp || LocalDensity.current.fontScale > 1.15f) 2 else 3
                Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                    actions.chunked(perRow).forEach { row ->
                        Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                            row.forEach { a -> ActionTile(a, Modifier.weight(1f)) }
                            repeat(perRow - row.size) { Spacer(Modifier.weight(1f)) }
                        }
                    }
                }
            }
        }
    }
}

@Composable
fun ActionTile(a: TileAction, modifier: Modifier = Modifier) {
    Column(
        modifier.heightIn(min = 76.dp).clip(RoundedCornerShape(14.dp)).background(T.surface2)
            .clickable(enabled = a.enabled, role = Role.Button, onClick = a.onClick)
            .disabledReason(a).padding(horizontal = 10.dp, vertical = 12.dp),
        verticalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        LIcon(a.icon, if (a.enabled) hue(a.h) else T.text3, 22.dp)
        Text(a.label, style = tileLabelStyle(), color = if (a.enabled) T.text else T.text2, maxLines = 2, overflow = TextOverflow.Ellipsis)
        if (!a.enabled && a.reason != null) {
            Text(a.reason, style = MaterialTheme.typography.bodySmall, color = T.text3, maxLines = 1, overflow = TextOverflow.Ellipsis)
        }
    }
}

/** A small stat: icon, big value, label and a muted line (Home). */
@Composable
fun StatCard(@DrawableRes icon: Int, h: Hue, value: String, label: String, sub: String?, modifier: Modifier = Modifier, onClick: (() -> Unit)? = null) {
    SoftCard(modifier.semantics(mergeDescendants = true) {}, onClick = onClick, padding = PaddingValues(12.dp)) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            LIcon(icon, hue(h), 20.dp)
            Spacer(Modifier.width(6.dp))
            Text(
                value, style = MaterialTheme.typography.titleLarge, color = T.text, fontWeight = FontWeight.Bold,
                maxLines = 1, softWrap = false, overflow = TextOverflow.Ellipsis,
            )
        }
        Column {
            Text(label, style = MaterialTheme.typography.labelMedium, color = T.text2, maxLines = 1, overflow = TextOverflow.Ellipsis)
            if (sub != null) Text(sub, style = MaterialTheme.typography.bodySmall, color = T.text3, maxLines = 1, overflow = TextOverflow.Ellipsis)
        }
    }
}

/** The bar at the top of the main tabs: the logo with the wordmark (Home)
 *  or the tab's [title], the bell (with how many messages are new) and
 *  Settings. */
@Composable
fun TopBar(title: String?, bellCount: Int, onBell: () -> Unit, onSettings: () -> Unit) {
    Row(
        Modifier.fillMaxWidth().statusBarsPadding().heightIn(min = 56.dp).padding(start = 16.dp, end = 4.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Image(painterResource(logoRes()), null, Modifier.size(if (title == null) 40.dp else 32.dp))
        Spacer(Modifier.width(10.dp))
        Box(Modifier.weight(1f)) {
            if (title == null) {
                Wordmark(fontSize = 20, maxLines = 1)
            } else {
                Text(
                    title, Modifier.semantics { heading() }, style = MaterialTheme.typography.titleLarge,
                    color = T.text, fontWeight = FontWeight.Bold, maxLines = 1, overflow = TextOverflow.Ellipsis,
                )
            }
        }
        val bellLabel = if (bellCount > 0) {
            androidx.compose.ui.res.pluralStringResource(R.plurals.top_bell_count, bellCount, bellCount)
        } else {
            stringResource(R.string.top_bell)
        }
        IconButton(onClick = onBell, modifier = Modifier.semantics { contentDescription = bellLabel }) {
            BadgedBox(badge = {
                if (bellCount > 0) {
                    // White on the dark themes' danger pink is 2.7:1: a deeper red.
                    Badge(containerColor = if (T.isLight) T.danger else Color(0xFFE11D48), contentColor = Color.White) {
                        Text(if (bellCount > 99) "99+" else "$bellCount", style = MaterialTheme.typography.labelMedium)
                    }
                }
            }) { LIcon(R.drawable.ic_lucide_bell, T.text, 24.dp) }
        }
        val settingsLabel = stringResource(R.string.tab_settings)
        IconButton(onClick = onSettings, modifier = Modifier.semantics { contentDescription = settingsLabel }) {
            LIcon(R.drawable.ic_lucide_settings, T.text, 24.dp)
        }
    }
}

/** A pushed screen's bar: Back, the title, and [actions]. */
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
