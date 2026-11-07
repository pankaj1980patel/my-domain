package com.mydomain.android.ui

import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.rememberScrollState
import androidx.compose.material3.FilterChip
import androidx.compose.material3.FilterChipDefaults
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.MutableState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.unit.dp
import com.mydomain.android.R
import com.mydomain.android.SecureMode
import com.mydomain.android.core.Core
import com.mydomain.android.core.SecureModePlan
import kotlinx.coroutines.delay

/** Secure mode now, looked at again every couple of seconds (you turn the
 *  helper on in Android's Settings, or it switches itself off). */
@Composable
private fun secureState(ctx: Context): MutableState<SecureModePlan.State> {
    val state = remember { mutableStateOf(SecureMode.state(ctx)) }
    LaunchedEffect(Unit) {
        while (true) {
            Core.awaitOnScreen()
            state.value = SecureMode.state(ctx)
            delay(2_000)
        }
    }
    return state
}

/** Settings → Secure mode: the phone helper off so banking apps work, the
 *  automatic options, and instant switching (a permission only you can
 *  grant, over adb). */
@Composable
fun SecureModeTile() {
    val ctx = LocalContext.current
    var state by secureState(ctx)
    Tile(title = stringResource(R.string.secure_title)) {
        Muted(stringResource(R.string.secure_hint))
        // Off with no instant switching: it stays on until you turn the
        // helper on in the Accessibility settings that open.
        SwitchRow(stringResource(R.string.secure_switch), SecureModePlan.shownOn(state)) {
            if (it) SecureMode.turnOn(ctx) else SecureMode.turnOff(ctx)
            state = SecureMode.state(ctx)
        }
        Text(
            stringResource(
                when (state) {
                    SecureModePlan.State.SECURE -> R.string.secure_state_on
                    SecureModePlan.State.TURNING_OFF -> R.string.secure_state_turning_off
                    SecureModePlan.State.HELPER_ON -> R.string.secure_state_helper_on
                    SecureModePlan.State.HELPER_OFF -> R.string.secure_state_helper_off
                },
            ),
            Modifier.announce(), style = MaterialTheme.typography.bodyMedium, color = T.text,
        )
        Muted(stringResource(R.string.secure_tile_hint))

        // Automatic: after N minutes with no remote control, and when a
        // remote-control session ends.
        var idle by remember { mutableStateOf(SecureMode.idleMinutes(ctx)) }
        Muted(stringResource(R.string.secure_auto_idle))
        IdleChips(idle) {
            idle = it
            SecureMode.setIdleMinutes(ctx, it)
        }
        var after by remember { mutableStateOf(SecureMode.afterSession(ctx)) }
        SwitchRow(stringResource(R.string.secure_auto_after), after) {
            after = it
            SecureMode.setAfterSession(ctx, it)
        }

        // Instant switching: WRITE_SECURE_SETTINGS, granted by you over adb.
        var granted by remember { mutableStateOf(SecureMode.canWriteSecure(ctx)) }
        var checked by remember { mutableStateOf(false) }
        Text(stringResource(R.string.secure_instant_title), style = MaterialTheme.typography.titleSmall, color = T.text)
        Muted(stringResource(R.string.secure_instant_hint))
        Text(SecureMode.ADB_GRANT, color = T.text, fontFamily = FontFamily.Monospace, style = MaterialTheme.typography.bodySmall)
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            GhostButton(stringResource(R.string.secure_copy_command), {
                ctx.getSystemService(ClipboardManager::class.java)
                    ?.setPrimaryClip(ClipData.newPlainText("adb", SecureMode.ADB_GRANT))
            }, small = true)
            GhostButton(stringResource(R.string.secure_check), {
                granted = SecureMode.canWriteSecure(ctx)
                checked = true
            }, small = true)
        }
        if (checked || granted) {
            Text(
                stringResource(if (granted) R.string.secure_instant_granted else R.string.secure_instant_not_granted),
                Modifier.announce(), style = MaterialTheme.typography.bodyMedium, color = T.text,
            )
        }
    }
}

/** Never / 5 / 15 / 60 minutes. */
@Composable
private fun IdleChips(selected: Int, onSelect: (Int) -> Unit) {
    val colors = FilterChipDefaults.filterChipColors(
        containerColor = T.surface2, labelColor = T.text2,
        selectedContainerColor = T.accentBg, selectedLabelColor = T.accentFg,
    )
    Row(Modifier.fillMaxWidth().horizontalScroll(rememberScrollState()), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
        SecureModePlan.IDLE_CHOICES.forEach { min ->
            val on = min == selected
            val label = if (min == 0) stringResource(R.string.secure_auto_never) else stringResource(R.string.secure_auto_minutes, min)
            FilterChip(
                selected = on, onClick = { onSelect(min) }, label = { Text(label) }, colors = colors,
                border = FilterChipDefaults.filterChipBorder(enabled = true, selected = on, borderColor = T.borderStrong, selectedBorderColor = T.accent),
            )
        }
    }
}

/** Home: while the helper is on, a reminder that banking apps may refuse
 *  to work beside it, with Secure mode one tap away (or hidden for good). */
@Composable
fun SecureModeHomeCard() {
    val ctx = LocalContext.current
    val state by secureState(ctx)
    var hidden by remember { mutableStateOf(SecureMode.cardHidden(ctx)) }
    if (hidden || state != SecureModePlan.State.HELPER_ON) return
    Tile(title = stringResource(R.string.secure_card_title)) {
        Muted(stringResource(R.string.secure_card_body))
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            SoftButton(stringResource(R.string.secure_card_turn_on), { SecureMode.turnOn(ctx) })
            GhostButton(stringResource(R.string.secure_card_hide), {
                SecureMode.hideCard(ctx)
                hidden = true
            })
        }
    }
}
