package com.mydomain.android.ui

import androidx.compose.animation.core.RepeatMode
import androidx.compose.animation.core.animateFloat
import androidx.compose.animation.core.infiniteRepeatable
import androidx.compose.animation.core.rememberInfiniteTransition
import androidx.compose.animation.core.tween
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.OutlinedTextFieldDefaults
import androidx.compose.material3.Surface
import androidx.compose.material3.Switch
import androidx.compose.material3.SwitchDefaults
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.withStyle
import androidx.annotation.StringRes
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.text.input.VisualTransformation
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.mydomain.android.R
import com.mydomain.android.core.Core
import com.mydomain.android.core.LinkState
import com.mydomain.android.core.initial
import com.mydomain.android.core.linkLabel
import com.mydomain.android.core.linkStateOf
import com.mydomain.android.core.niceName
import com.mydomain.android.core.qualityLabel
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import com.mydomain.android.ffi.PeerInfo

/** A glass card with a hairline border: the one container style (kit card:
 *  18 dp corners). */
@Composable
fun Tile(
    modifier: Modifier = Modifier,
    title: String? = null,
    onClick: (() -> Unit)? = null,
    content: @Composable ColumnScope.() -> Unit,
) {
    Surface(
        modifier = modifier.fillMaxWidth(),
        shape = MaterialTheme.shapes.medium,
        color = T.surface,
        border = BorderStroke(1.dp, T.border),
        onClick = onClick ?: {},
        enabled = onClick != null,
    ) {
        Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(10.dp)) {
            if (title != null) Text(title, style = MaterialTheme.typography.titleMedium, color = T.text)
            content()
        }
    }
}

@Composable
fun Muted(text: String, modifier: Modifier = Modifier, center: Boolean = false) {
    Text(
        text, modifier = modifier, style = MaterialTheme.typography.bodySmall, color = T.text2,
        textAlign = if (center) TextAlign.Center else null,
    )
}

/** Primary action: the brand gradient with a white label (kit .btn). */
@Composable
fun PrimaryButton(text: String, onClick: () -> Unit, modifier: Modifier = Modifier, enabled: Boolean = true) {
    val shape = MaterialTheme.shapes.small
    Button(
        onClick,
        modifier = modifier.height(44.dp).alpha(if (enabled) 1f else 0.5f).clip(shape).background(T.brush, shape),
        enabled = enabled, shape = shape,
        colors = ButtonDefaults.buttonColors(
            containerColor = Color.Transparent, contentColor = T.onPrimary,
            disabledContainerColor = Color.Transparent, disabledContentColor = T.onPrimary,
        ),
        elevation = null, contentPadding = PaddingValues(horizontal = 18.dp),
    ) { Text(text, style = MaterialTheme.typography.labelLarge, fontWeight = FontWeight.ExtraBold) }
}

/** Secondary action: deep fill with a hairline (kit .btn.secondary). */
@Composable
fun SoftButton(text: String, onClick: () -> Unit, modifier: Modifier = Modifier, enabled: Boolean = true) {
    Button(
        onClick, modifier = modifier.height(44.dp), enabled = enabled, shape = MaterialTheme.shapes.small,
        colors = ButtonDefaults.buttonColors(containerColor = T.secondaryButton, contentColor = T.text),
        border = BorderStroke(1.dp, T.secondaryButtonBorder),
        elevation = null, contentPadding = PaddingValues(horizontal = 16.dp),
    ) { Text(text, style = MaterialTheme.typography.labelLarge) }
}

@Composable
fun GhostButton(text: String, onClick: () -> Unit, modifier: Modifier = Modifier, small: Boolean = false) {
    OutlinedButton(
        onClick, modifier = modifier.height(if (small) 36.dp else 44.dp), shape = MaterialTheme.shapes.small,
        border = BorderStroke(1.dp, T.borderStrong),
        colors = ButtonDefaults.outlinedButtonColors(contentColor = T.text),
        contentPadding = PaddingValues(horizontal = if (small) 12.dp else 16.dp),
    ) { Text(text, style = if (small) MaterialTheme.typography.labelMedium else MaterialTheme.typography.labelLarge) }
}

@Composable
fun Field(
    value: String,
    onChange: (String) -> Unit,
    label: String,
    modifier: Modifier = Modifier,
    password: Boolean = false,
    singleLine: Boolean = true,
) {
    OutlinedTextField(
        value, onChange, modifier = modifier.fillMaxWidth(), label = { Text(label) }, singleLine = singleLine,
        shape = MaterialTheme.shapes.small,
        visualTransformation = if (password) PasswordVisualTransformation() else VisualTransformation.None,
        colors = OutlinedTextFieldDefaults.colors(
            focusedBorderColor = T.accent, unfocusedBorderColor = T.border,
            focusedLabelColor = T.accentFg, unfocusedLabelColor = T.text3, cursorColor = T.accent,
            focusedContainerColor = T.surface2, unfocusedContainerColor = T.surface2,
            focusedTextColor = T.text, unfocusedTextColor = T.text,
        ),
    )
}

@Composable
fun SwitchRow(label: String, checked: Boolean, onChange: (Boolean) -> Unit) {
    Row(
        Modifier.fillMaxWidth().clickable { onChange(!checked) }.padding(vertical = 4.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Text(label, Modifier.weight(1f), style = MaterialTheme.typography.bodyMedium, color = T.text)
        Switch(
            checked, onChange,
            colors = SwitchDefaults.colors(
                checkedTrackColor = T.accent, checkedThumbColor = T.onPrimary, checkedBorderColor = T.accent,
                uncheckedTrackColor = T.surface2, uncheckedBorderColor = T.borderStrong, uncheckedThumbColor = T.text2,
            ),
        )
    }
}

@Composable
fun Avatar(name: String, size: Dp = 44.dp) {
    Box(
        Modifier.size(size).background(T.surface2, MaterialTheme.shapes.small),
        contentAlignment = Alignment.Center,
    ) {
        Text(initial(name), color = T.text, fontWeight = FontWeight.SemiBold, fontSize = (size.value * 0.4f).sp)
    }
}

/** Coloured dot + plain-words status. */
@Composable
fun StatusPill(node: String) {
    val s = linkStateOf(node)
    val color = when (s) {
        LinkState.Direct -> T.success
        LinkState.Relay -> T.warning
        LinkState.Failed -> T.danger
        LinkState.Connecting -> T.accent
        LinkState.Offline -> T.text3
    }
    val pulse = if (s == LinkState.Connecting) {
        val tr = rememberInfiniteTransition(label = "pulse")
        val a by tr.animateFloat(0.3f, 1f, infiniteRepeatable(tween(700), RepeatMode.Reverse), label = "a")
        a
    } else 1f
    val q = if (s == LinkState.Direct || s == LinkState.Relay) Core.quality[node] else null
    Row(verticalAlignment = Alignment.CenterVertically) {
        // Connected dots glow softly (kit .dot), except in the quiet theme.
        val glow = s == LinkState.Direct && T.dotGlow
        Box(
            Modifier.size(8.dp).alpha(pulse)
                .drawBehind { if (glow) drawCircle(color.copy(alpha = 0.3f), radius = size.minDimension) }
                .background(color, CircleShape),
        )
        Spacer(Modifier.width(6.dp))
        Text(
            linkLabel(s), style = MaterialTheme.typography.labelMedium,
            color = if (s == LinkState.Offline || s == LinkState.Connecting) T.text2 else color,
        )
        if (q != null && qualityLabel(q) != null) {
            Spacer(Modifier.width(6.dp))
            SignalBars(q, color)
        }
    }
}

/** Three bars, filled by link quality. */
@Composable
fun SignalBars(quality: String, color: androidx.compose.ui.graphics.Color) {
    val filled = when (quality) { "excellent" -> 3; "good" -> 2; "weak" -> 1; else -> 0 }
    val on = if (quality == "weak") T.warning else color
    Row(
        Modifier.semantics { contentDescription = qualityLabel(quality) ?: "" },
        verticalAlignment = Alignment.Bottom,
        horizontalArrangement = Arrangement.spacedBy(2.dp),
    ) {
        listOf(5, 8, 12).forEachIndexed { i, h ->
            Box(
                Modifier.width(3.dp).height(h.dp)
                    .background(if (i < filled) on else T.borderStrong, RoundedCornerShape(1.dp)),
            )
        }
    }
}

@Composable
fun DeviceCard(peer: PeerInfo, onOpen: () -> Unit, onConnect: () -> Unit) {
    val s = linkStateOf(peer.nodeId)
    val unread = Core.messages.count { it.node == peer.nodeId && it.incoming }
    Tile(onClick = onOpen) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Avatar(peer.name)
            Spacer(Modifier.width(12.dp))
            Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(2.dp)) {
                Text(niceName(peer.name), style = MaterialTheme.typography.titleMedium, color = T.text)
                StatusPill(peer.nodeId)
            }
            if (unread > 0) {
                Box(
                    Modifier.padding(start = 8.dp).background(T.accent, CircleShape).padding(horizontal = 8.dp, vertical = 2.dp),
                ) { Text("$unread", color = T.onAccent, fontSize = 11.sp, fontWeight = FontWeight.ExtraBold) }
            }
            if (s == LinkState.Offline || s == LinkState.Failed) {
                Spacer(Modifier.width(8.dp))
                GhostButton(stringResource(R.string.connect), onConnect, small = true)
            }
        }
    }
}

@Composable
fun EmptyState(title: String, body: String, modifier: Modifier = Modifier) {
    Column(
        modifier.fillMaxWidth().padding(vertical = 48.dp, horizontal = 24.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.spacedBy(6.dp),
    ) {
        Text(title, style = MaterialTheme.typography.titleMedium, color = T.text)
        Muted(body, center = true)
    }
}

@Composable
fun ErrorText(text: String) {
    Text(
        text, Modifier.fillMaxWidth().background(T.dangerSoft, MaterialTheme.shapes.extraSmall).padding(10.dp),
        color = T.danger, style = MaterialTheme.typography.bodySmall,
    )
}

/** Page header: big title, optional muted line, optional trailing action. */
@Composable
fun PageHead(title: String, subtitle: String? = null, action: (@Composable () -> Unit)? = null) {
    Row(Modifier.fillMaxWidth().padding(bottom = 4.dp), verticalAlignment = Alignment.CenterVertically) {
        Column(Modifier.weight(1f)) {
            Text(title, style = MaterialTheme.typography.headlineSmall, color = T.text)
            if (subtitle != null) Muted(subtitle)
        }
        action?.invoke()
    }
}


/** A string resource with its one `%1$s` argument in bold (the words around
 *  it stay one translatable sentence). */
@Composable
fun boldArg(@StringRes id: Int, arg: String): AnnotatedString {
    val parts = stringResource(id).split("%1\$s", limit = 2)
    return buildAnnotatedString {
        append(parts[0])
        withStyle(SpanStyle(fontWeight = FontWeight.SemiBold)) { append(arg) }
        if (parts.size > 1) append(parts[1])
    }
}
