package com.mydomain.android.ui

import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
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
import androidx.compose.ui.geometry.CornerRadius
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.Path
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.StrokeJoin
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.role
import androidx.compose.ui.semantics.selected
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.mydomain.android.R
import com.mydomain.android.core.Core
import com.mydomain.android.core.DeviceLabels
import com.mydomain.android.core.initial
import com.mydomain.android.core.niceName

// A device's look on this phone: its avatar (the icon you chose, else the
// initial of its name, ringed in the colour you chose) and the editor that
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
    else -> id
}

@Composable
private fun colorName(id: String): String = when (id) {
    "cyan" -> stringResource(R.string.label_color_cyan)
    "blue" -> stringResource(R.string.label_color_blue)
    "sky" -> stringResource(R.string.label_color_sky)
    "green" -> stringResource(R.string.label_color_green)
    "gold" -> stringResource(R.string.label_color_gold)
    "rose" -> stringResource(R.string.label_color_rose)
    "grey" -> stringResource(R.string.label_color_grey)
    else -> id
}

/** One device icon, line-drawn on a 24-unit grid (the desktop's SVGs). */
@Composable
fun DeviceIcon(icon: String, size: Dp = 24.dp, color: Color = T.text) {
    Canvas(Modifier.size(size)) {
        val u = this.size.minDimension / 24f
        val width = 1.8f * u
        val stroke = Stroke(width = width, cap = StrokeCap.Round, join = StrokeJoin.Round)
        val rect = { x: Float, y: Float, w: Float, h: Float, r: Float ->
            drawRoundRect(color, Offset(x * u, y * u), Size(w * u, h * u), CornerRadius(r * u), style = stroke)
        }
        val line = { x1: Float, y1: Float, x2: Float, y2: Float ->
            drawLine(color, Offset(x1 * u, y1 * u), Offset(x2 * u, y2 * u), width, StrokeCap.Round)
        }
        val poly = { pts: List<Float> ->
            val p = Path()
            p.moveTo(pts[0] * u, pts[1] * u)
            var i = 2
            while (i + 1 < pts.size) {
                p.lineTo(pts[i] * u, pts[i + 1] * u)
                i += 2
            }
            drawPath(p, color, style = stroke)
        }
        when (icon) {
            "phone" -> { rect(7f, 2f, 10f, 20f, 2f); line(11f, 18f, 13f, 18f) }
            "tablet" -> { rect(4f, 2f, 16f, 20f, 2f); line(11f, 18f, 13f, 18f) }
            "laptop" -> { rect(4f, 5f, 16f, 11f, 1.5f); line(2f, 19f, 22f, 19f) }
            "desktop" -> { rect(3f, 4f, 18f, 12f, 1.5f); line(8f, 20f, 16f, 20f); line(12f, 16f, 12f, 20f) }
            "tv" -> { rect(2f, 7f, 20f, 13f, 2f); poly(listOf(8f, 3f, 12f, 7f, 16f, 3f)) }
            "watch" -> {
                rect(6f, 6f, 12f, 12f, 3f)
                poly(listOf(9f, 6f, 10f, 2f, 14f, 2f, 15f, 6f))
                poly(listOf(9f, 18f, 10f, 22f, 14f, 22f, 15f, 18f))
            }
            else -> {}
        }
    }
}

/** A device's avatar: its icon (else its initial), ringed and tinted in
 *  its colour. The icon or letter stays in the text colour, so it keeps
 *  full contrast whatever the colour. */
@Composable
fun DeviceAvatar(node: String, size: Dp = 44.dp) {
    val label = DeviceLabels.labels[node]
    val own = Core.peers.firstOrNull { it.nodeId == node }?.name.orEmpty()
    val accent = labelColor(label?.color)
    val shape = MaterialTheme.shapes.small
    var m = Modifier.size(size).clip(shape).background(T.surface2, shape)
    if (accent != null) m = m.background(accent.copy(alpha = 0.18f), shape).border(BorderStroke(2.dp, accent), shape)
    Box(m, contentAlignment = Alignment.Center) {
        val icon = label?.icon
        if (icon != null) {
            DeviceIcon(icon, size * 0.55f)
        } else {
            Text(initial(label?.nickname ?: own), color = T.text, fontWeight = FontWeight.SemiBold, fontSize = (size.value * 0.4f).sp)
        }
    }
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
        Muted(String.format(stringResource(R.string.label_hint), own))
        Field(nickname, { nickname = it.take(DeviceLabels.MAX_NICKNAME) }, stringResource(R.string.label_nickname))
        if (nickname.isEmpty()) Muted(String.format(stringResource(R.string.label_nickname_blank), own))

        Text(stringResource(R.string.label_icon), style = MaterialTheme.typography.labelLarge, color = T.text2)
        Row(Modifier.horizontalScroll(rememberScrollState()), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            Choice(icon == null, stringResource(R.string.label_none), { icon = null }) {
                Text(initial(nickname.ifEmpty { own }), color = T.text, fontWeight = FontWeight.SemiBold)
            }
            DeviceLabels.ICONS.forEach { id ->
                Choice(icon == id, iconName(id), { icon = id }) { DeviceIcon(id) }
            }
        }

        Text(stringResource(R.string.label_color), style = MaterialTheme.typography.labelLarge, color = T.text2)
        Row(Modifier.horizontalScroll(rememberScrollState()), horizontalArrangement = Arrangement.spacedBy(10.dp)) {
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

/** An icon choice: a square with a ring when chosen. */
@Composable
private fun Choice(on: Boolean, name: String, onClick: () -> Unit, content: @Composable () -> Unit) {
    val shape = MaterialTheme.shapes.small
    Box(
        Modifier.size(44.dp).clip(shape).background(T.surface2, shape)
            .border(BorderStroke(if (on) 2.dp else 1.dp, if (on) T.accent else T.border), shape)
            .clickable(onClick = onClick)
            .semantics { contentDescription = name; selected = on; role = Role.RadioButton },
        contentAlignment = Alignment.Center,
    ) { content() }
}

/** A colour choice: a round swatch (none: an empty one with a slash), with
 *  a ring in the focus colour when chosen. */
@Composable
private fun Swatch(fill: Color?, on: Boolean, name: String, onClick: () -> Unit) {
    Box(
        Modifier.size(40.dp).clip(CircleShape)
            .border(BorderStroke(2.dp, if (on) T.accent else Color.Transparent), CircleShape)
            .clickable(onClick = onClick)
            .semantics { contentDescription = name; selected = on; role = Role.RadioButton },
        contentAlignment = Alignment.Center,
    ) {
        val inner = Modifier.size(30.dp).clip(CircleShape).background(fill ?: T.surface2, CircleShape)
            .border(BorderStroke(1.dp, T.borderStrong), CircleShape)
        if (fill != null) {
            Box(inner)
        } else {
            val slash = T.text3
            Box(inner) {
                Canvas(Modifier.size(30.dp)) {
                    drawLine(slash, Offset(size.width * 0.25f, size.height * 0.75f), Offset(size.width * 0.75f, size.height * 0.25f), 2.dp.toPx(), StrokeCap.Round)
                }
            }
        }
    }
}
