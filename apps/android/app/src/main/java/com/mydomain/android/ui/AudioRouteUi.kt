package com.mydomain.android.ui

import com.mydomain.android.core.AppScope
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.selection.selectable
import androidx.compose.foundation.selection.selectableGroup
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.RadioButton
import androidx.compose.material3.RadioButtonDefaults
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.unit.dp
import com.mydomain.android.ComputerAudio
import com.mydomain.android.MuteHere
import com.mydomain.android.R
import com.mydomain.android.core.AudioRoutes
import com.mydomain.android.core.Core
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.withContext

/** "Send audio over": Auto (Wi-Fi first, Bluetooth when offline), Wi-Fi or
 *  internet only, Bluetooth only. Kept per device by the core; a listen
 *  running here tells the computer at once. */
@Composable
fun AudioRoutePicker(node: String) {
    var route by remember(node) {
        mutableStateOf(AudioRoutes.of(runCatching { Core.engine()?.audioRoute(node) }.getOrNull()))
    }
    val labels = mapOf(
        AudioRoutes.AUTO to stringResource(R.string.audio_route_auto),
        AudioRoutes.WIFI to stringResource(R.string.audio_route_wifi),
        AudioRoutes.BLUETOOTH to stringResource(R.string.audio_route_bluetooth),
    )
    Muted(stringResource(R.string.audio_route_label))
    Column(Modifier.selectableGroup()) {
        AudioRoutes.ALL.forEach { r ->
            Row(
                Modifier.fillMaxWidth().heightIn(min = 48.dp).selectable(
                    selected = route == r, role = Role.RadioButton,
                    onClick = {
                        route = r
                        AppScope.bg("audio route") {
                            runCatching {
                                val e = Core.engine() ?: return@runCatching
                                e.setAudioRoute(node, r)
                                if (ComputerAudio.playingFrom == node) e.audioListen(node, true)
                            }
                        }
                    },
                ),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                RadioButton(
                    selected = route == r, onClick = null,
                    colors = RadioButtonDefaults.colors(selectedColor = T.accent),
                )
                Text(
                    labels[r].orEmpty(), Modifier.padding(start = 8.dp),
                    style = MaterialTheme.typography.bodyMedium, color = T.text,
                )
            }
        }
    }
}

/** "via Bluetooth" while the stream with [node] goes over Bluetooth (no
 *  network link): lower quality, a little more delay. */
@Composable
fun ViaBluetoothBadge(node: String) {
    var via by remember(node) { mutableStateOf(false) }
    LaunchedEffect(node) {
        while (true) {
            Core.awaitOnScreen()
            via = AudioRoutes.viaBluetooth(
                withContext(Dispatchers.IO) { runCatching { Core.engine()?.audioVia(node) }.getOrNull() },
            )
            delay(1_000)
        }
    }
    if (!via) return
    Column {
        Text(
            stringResource(R.string.audio_route_badge),
            modifier = Modifier
                .background(T.successSoft, RoundedCornerShape(50))
                .padding(horizontal = 8.dp, vertical = 2.dp),
            style = MaterialTheme.typography.labelSmall,
            color = T.success,
        )
        Muted(stringResource(R.string.audio_route_badge_hint))
    }
}

/** "Mute this phone while it plays on [name]": a switch per computer. */
@Composable
fun MuteHereSwitch(node: String, name: String) {
    val ctx = LocalContext.current
    var on by remember(node) { mutableStateOf(MuteHere.switchOn(ctx, node)) }
    SwitchRow(stringResource(R.string.mute_here_label, name), on) {
        on = it
        MuteHere.setSwitch(ctx, node, it)
    }
    if (MuteHere.active && on) Muted(stringResource(R.string.mute_here_active))
}
