package com.mydomain.android.ui

import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.foundation.selection.selectableGroup
import androidx.compose.foundation.selection.selectable
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.Path
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.StrokeJoin
import androidx.compose.ui.graphics.PathEffect
import androidx.compose.ui.graphics.drawscope.DrawScope
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.graphics.drawscope.scale
import androidx.compose.ui.graphics.drawscope.translate
import androidx.compose.ui.graphics.vector.PathParser
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.role
import androidx.compose.ui.semantics.selected
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import com.mydomain.android.R
import com.mydomain.android.core.Core
import com.mydomain.android.core.DeviceIconShapes
import com.mydomain.android.core.DeviceKindPlan
import com.mydomain.android.core.DeviceKinds
import com.mydomain.android.core.DeviceLabels
import com.mydomain.android.core.niceName

// A device's look on this phone: its avatar (the icon you chose, else the
// one for what it is, ringed in the colour you chose) and the editor that
// sets the nickname, icon and colour ([DeviceLabels]; names come from
// Core.peerName, which prefers the nickname). Same sets as the desktop.

/** The theme colour an accent colour id stands for (null: none). */
@Composable
fun labelColor(id: String?): Color? = when (id) {
    "cyan" -> T.accent
    "blue" -> T.primary
    "sky" -> T.accentSoft
    "green" -> T.success
    "gold" -> T.warning
    "rose" -> T.danger
    "grey" -> T.text3
    else -> null
}

@Composable
private fun iconName(id: String): String = when (id) {
    "phone" -> stringResource(R.string.label_icon_phone)
    "tablet" -> stringResource(R.string.label_icon_tablet)
    "laptop" -> stringResource(R.string.label_icon_laptop)
    "desktop" -> stringResource(R.string.label_icon_desktop)
    "tv" -> stringResource(R.string.label_icon_tv)
    "watch" -> stringResource(R.string.label_icon_watch)
    "android_phone" -> stringResource(R.string.label_icon_android_phone)
    "android_tablet" -> stringResource(R.string.label_icon_android_tablet)
    "android_foldable" -> stringResource(R.string.label_icon_android_foldable)
    "mac_laptop" -> stringResource(R.string.label_icon_mac_laptop)
    "imac" -> stringResource(R.string.label_icon_imac)
    "mac_desktop" -> stringResource(R.string.label_icon_mac_desktop)
    "win_laptop" -> stringResource(R.string.label_icon_win_laptop)
    "win_desktop" -> stringResource(R.string.label_icon_win_desktop)
    "linux_laptop" -> stringResource(R.string.label_icon_linux_laptop)
    "linux_desktop" -> stringResource(R.string.label_icon_linux_desktop)
    "unknown" -> stringResource(R.string.label_icon_unknown)
    else -> id
}

@Composable
internal fun colorName(id: String): String = when (id) {
    "cyan" -> stringResource(R.string.label_color_cyan)
    "blue" -> stringResource(R.string.label_color_blue)
    "sky" -> stringResource(R.string.label_color_sky)
    "green" -> stringResource(R.string.label_color_green)
    "gold" -> stringResource(R.string.label_color_gold)
    "rose" -> stringResource(R.string.label_color_rose)
    "grey" -> stringResource(R.string.label_color_grey)
    else -> id
}

/** The parsed paths of one icon, kept per icon name. */
private val parsedIcons = HashMap<String, List<Pair<DeviceIconShapes.Part, Path>>>()
private val parsedMarks = HashMap<String, List<Pair<DeviceIconShapes.Part, Path>>>()

private fun parsed(parts: List<DeviceIconShapes.Part>): List<Pair<DeviceIconShapes.Part, Path>> =
    parts.map { it to PathParser().parsePathString(it.d).toPath() }

private fun partsOf(icon: String): List<Pair<DeviceIconShapes.Part, Path>> =
    synchronized(parsedIcons) { parsedIcons.getOrPut(icon) { parsed(DeviceIconShapes.shape(icon).parts) } }

private fun markOf(kind: String): List<Pair<DeviceIconShapes.Part, Path>> =
    synchronized(parsedMarks) { parsedMarks.getOrPut(kind) { parsed(DeviceIconShapes.markParts(kind)) } }

private val iconDash = PathEffect.dashPathEffect(floatArrayOf(1.6f, 1.6f))

private fun DrawScope.drawParts(list: List<Pair<DeviceIconShapes.Part, Path>>, color: Color) {
    for ((part, path) in list) {
        if (part.fill) {
            drawPath(path, color)
        } else {
            val stroke = Stroke(
                width = if (part.thin) 0.9f else 1.8f,
                cap = StrokeCap.Round,
                join = StrokeJoin.Round,
                pathEffect = if (part.dashed) iconDash else null,
            )
            drawPath(path, color, style = stroke)
        }
    }
}

/** Draw device icon [icon] ([DeviceIconShapes]: the desktop's SVGs) in a
 *  [px]-wide square at [topLeft]: the device's shape with its OS mark. */
fun DrawScope.drawDeviceIcon(icon: String, topLeft: Offset, px: Float, color: Color) {
    val shape = DeviceIconShapes.shape(icon)
    translate(topLeft.x, topLeft.y) {
        scale(px / 24f, pivot = Offset.Zero) {
            drawParts(partsOf(icon), color)
            shape.mark?.let { m ->
                translate(m.x, m.y) {
                    scale(m.scale, pivot = Offset.Zero) { drawParts(markOf(m.kind), color) }
                }
            }
        }
    }
}

/** One device icon, drawn on a 24-unit grid in [color]. */
@Composable
fun DeviceIcon(icon: String, size: Dp = 24.dp, color: Color = T.text) {
    Canvas(Modifier.size(size)) { drawDeviceIcon(icon, Offset.Zero, this.size.minDimension, color) }
}

/** The nickname / icon / colour editor, for a device's screen. */
@Composable
fun DeviceLabelEditor(node: String, onDone: () -> Unit) {
    val current = DeviceLabels.labels[node]
    val own = niceName(Core.peers.firstOrNull { it.nodeId == node }?.name.orEmpty())
    var nickname by remember(node) { mutableStateOf(current?.nickname.orEmpty()) }
    var icon by remember(node) { mutableStateOf(current?.icon) }
    var color by remember(node) { mutableStateOf(current?.color) }
    val toast = LocalToast.current
    val saved = stringResource(R.string.label_saved)

    Tile(title = stringResource(R.string.label_title)) {
        Muted(stringResource(R.string.label_hint))
        Field(nickname, { nickname = it.take(DeviceLabels.MAX_NICKNAME) }, stringResource(R.string.label_nickname))
        if (nickname.isEmpty()) Muted(String.format(stringResource(R.string.label_nickname_blank), own))

        Text(stringResource(R.string.label_icon), style = MaterialTheme.typography.labelLarge, color = T.text2)
        Row(Modifier.selectableGroup().horizontalScroll(rememberScrollState()), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            // Automatic: the icon for what it is (a Mac laptop, an Android phone...).
            Choice(icon == null, stringResource(R.string.label_icon_auto), { icon = null }) {
                DeviceIcon(DeviceKindPlan.iconOf(null, DeviceKinds.kinds[node]))
            }
            DeviceLabels.ICONS.forEach { id ->
                Choice(icon == id, iconName(id), { icon = id }) { DeviceIcon(id) }
            }
        }

        Text(stringResource(R.string.label_color), style = MaterialTheme.typography.labelLarge, color = T.text2)
        Row(Modifier.selectableGroup().horizontalScroll(rememberScrollState()), horizontalArrangement = Arrangement.spacedBy(6.dp)) {
            Swatch(null, color == null, stringResource(R.string.label_none)) { color = null }
            DeviceLabels.COLORS.forEach { id ->
                Swatch(labelColor(id), color == id, colorName(id)) { color = id }
            }
        }

        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            PrimaryButton(stringResource(R.string.label_save), {
                Core.setDeviceLabel(node, nickname, icon, color)
                toast(saved)
                onDone()
            })
            GhostButton(stringResource(R.string.label_cancel), onDone)
            if (current != null) {
                GhostButton(stringResource(R.string.label_reset), {
                    Core.setDeviceLabel(node, null, null, null)
                    toast(saved)
                    onDone()
                })
            }
        }
    }
}

/** An icon choice: a square with a ring when chosen (48 dp: a full
 *  touch target). */
@Composable
private fun Choice(on: Boolean, name: String, onClick: () -> Unit, content: @Composable () -> Unit) {
    val shape = MaterialTheme.shapes.small
    Box(
        Modifier.size(48.dp).clip(shape).background(T.surface2, shape)
            .border(BorderStroke(if (on) 2.dp else 1.dp, if (on) T.accent else T.controlBorder), shape)
            .selectable(selected = on, role = Role.RadioButton, onClick = onClick)
            .clearAndSetSemantics { contentDescription = name; selected = on; role = Role.RadioButton },
        contentAlignment = Alignment.Center,
    ) { content() }
}

/** A colour choice: a round swatch (none: an empty one with a slash), with
 *  a ring in the focus colour when chosen. */
@Composable
internal fun Swatch(fill: Color?, on: Boolean, name: String, onClick: () -> Unit) {
    Box(
        Modifier.size(48.dp).clip(CircleShape)
            .border(BorderStroke(2.dp, if (on) T.accent else Color.Transparent), CircleShape)
            .selectable(selected = on, role = Role.RadioButton, onClick = onClick)
            .clearAndSetSemantics { contentDescription = name; selected = on; role = Role.RadioButton },
        contentAlignment = Alignment.Center,
    ) {
        val inner = Modifier.size(36.dp).clip(CircleShape).background(fill ?: T.surface2, CircleShape)
            .border(BorderStroke(1.dp, T.borderStrong), CircleShape)
        if (fill != null) {
            Box(inner)
        } else {
            val slash = T.text3
            Box(inner) {
                Canvas(Modifier.size(36.dp)) {
                    drawLine(slash, Offset(size.width * 0.25f, size.height * 0.75f), Offset(size.width * 0.75f, size.height * 0.25f), 2.dp.toPx(), StrokeCap.Round)
                }
            }
        }
    }
}
