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
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.text.input.VisualTransformation
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.mydomain.android.core.Core
import com.mydomain.android.core.LinkState
import com.mydomain.android.core.initial
import com.mydomain.android.core.linkLabel
import com.mydomain.android.core.linkStateOf
import com.mydomain.android.core.niceName
import com.mydomain.android.ffi.PeerInfo

/** A flat card with a hairline border: the one container style. */
@Composable
fun Tile(
    modifier: Modifier = Modifier,
    title: String? = null,
    onClick: (() -> Unit)? = null,
    content: @Composable ColumnScope.() -> Unit,
) {
    Surface(
        modifier = modifier.fillMaxWidth(),
        shape = RoundedCornerShape(14.dp),
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

@Composable
fun PrimaryButton(text: String, onClick: () -> Unit, modifier: Modifier = Modifier, enabled: Boolean = true) {
    Button(
        onClick, modifier = modifier.height(44.dp), enabled = enabled, shape = RoundedCornerShape(10.dp),
        colors = ButtonDefaults.buttonColors(containerColor = T.accent, contentColor = T.accentText),
        contentPadding = PaddingValues(horizontal = 18.dp),
    ) { Text(text, style = MaterialTheme.typography.labelLarge) }
}

/** Secondary action: soft grey fill, no border. */
@Composable
fun SoftButton(text: String, onClick: () -> Unit, modifier: Modifier = Modifier, enabled: Boolean = true) {
    Button(
        onClick, modifier = modifier.height(44.dp), enabled = enabled, shape = RoundedCornerShape(10.dp),
        colors = ButtonDefaults.buttonColors(containerColor = T.surface2, contentColor = T.text),
        elevation = null, contentPadding = PaddingValues(horizontal = 16.dp),
    ) { Text(text, style = MaterialTheme.typography.labelLarge) }
}

@Composable
fun GhostButton(text: String, onClick: () -> Unit, modifier: Modifier = Modifier, small: Boolean = false) {
    OutlinedButton(
        onClick, modifier = modifier.height(if (small) 36.dp else 44.dp), shape = RoundedCornerShape(10.dp),
        border = BorderStroke(1.dp, T.border),
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
        shape = RoundedCornerShape(10.dp),
        visualTransformation = if (password) PasswordVisualTransformation() else VisualTransformation.None,
        colors = OutlinedTextFieldDefaults.colors(
            focusedBorderColor = T.text2, unfocusedBorderColor = T.border,
            focusedLabelColor = T.text2, cursorColor = T.text,
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
                checkedTrackColor = T.accent, checkedThumbColor = T.accentText,
                uncheckedTrackColor = T.surface2, uncheckedBorderColor = T.border,
            ),
        )
    }
}

@Composable
fun Avatar(name: String, size: Dp = 44.dp) {
    Box(
        Modifier.size(size).background(T.surface2, RoundedCornerShape(12.dp)),
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
    Row(verticalAlignment = Alignment.CenterVertically) {
        Box(Modifier.size(8.dp).alpha(pulse).background(color, CircleShape))
        Spacer(Modifier.width(6.dp))
        Text(
            linkLabel(s), style = MaterialTheme.typography.labelMedium,
            color = if (s == LinkState.Offline || s == LinkState.Connecting) T.text2 else color,
        )
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
                ) { Text("$unread", color = T.accentText, fontSize = 12.sp, fontWeight = FontWeight.Bold) }
            }
            if (s == LinkState.Offline || s == LinkState.Failed) {
                Spacer(Modifier.width(8.dp))
                GhostButton("Connect", onConnect, small = true)
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
        text, Modifier.fillMaxWidth().background(T.dangerSoft, RoundedCornerShape(8.dp)).padding(10.dp),
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

