package com.mydomain.android.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.pluralStringResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import com.mydomain.android.R
import com.mydomain.android.core.KeyRotation
import com.mydomain.android.core.RotationDeviceInfo
import com.mydomain.android.core.friendlyError
import kotlinx.coroutines.launch

/**
 * Home (and Settings): this phone missed a passphrase change made on
 * another device. Type the new passphrase, or "Not now".
 */
@Composable
fun KeyRotationPrompt(modifier: Modifier = Modifier) {
    LaunchedEffect(Unit) { runCatching { KeyRotation.load() } }
    val info = KeyRotation.info
    if (!info.needsPassphrase) return
    val toast = LocalToast.current
    val scope = rememberCoroutineScope()
    var pass by remember { mutableStateOf("") }
    val saved = stringResource(R.string.key_rotation_typed_saved)
    Tile(modifier, title = stringResource(R.string.key_rotation_needs_title)) {
        Text(
            stringResource(R.string.key_rotation_needs_body, info.mismatchName),
            Modifier.announce(urgent = true),
            style = MaterialTheme.typography.bodyMedium, color = T.text,
        )
        Field(pass, { pass = it }, stringResource(R.string.key_rotation_new), password = true)
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            PrimaryButton(stringResource(R.string.save), {
                scope.launch {
                    try {
                        KeyRotation.typed(pass)
                        pass = ""
                        toast(saved)
                    } catch (e: Exception) {
                        toast(friendlyError(e))
                    }
                }
            }, enabled = pass.isNotBlank())
            GhostButton(stringResource(R.string.key_rotation_not_now), { scope.launch { runCatching { KeyRotation.dismiss() } } })
        }
    }
}

@Composable
private fun statusText(d: RotationDeviceInfo): String = stringResource(
    when (d.status) {
        "waiting" -> R.string.key_rotation_status_waiting
        "ready" -> R.string.key_rotation_status_ready
        "updated" -> R.string.key_rotation_status_updated
        "expired" -> R.string.key_rotation_status_expired
        else -> R.string.key_rotation_status_no_key
    },
)

@Composable
private fun DeviceList(devices: List<RotationDeviceInfo>) {
    if (devices.isEmpty()) {
        Muted(stringResource(R.string.key_rotation_no_devices))
        return
    }
    for (d in devices) {
        Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            Text(d.name, Modifier.weight(1f), style = MaterialTheme.typography.bodyLarge, color = T.text)
            Muted(statusText(d))
        }
    }
}

/** Settings: change the passphrase on all devices, and follow the change. */
@Composable
fun KeyRotationTile(modifier: Modifier = Modifier) {
    LaunchedEffect(Unit) { runCatching { KeyRotation.load() } }
    val toast = LocalToast.current
    val scope = rememberCoroutineScope()
    val info = KeyRotation.info
    var open by remember { mutableStateOf(false) }
    var first by remember { mutableStateOf("") }
    var second by remember { mutableStateOf("") }
    var strength by remember { mutableIntStateOf(0) }
    var busy by remember { mutableStateOf(false) }
    LaunchedEffect(first) { strength = runCatching { KeyRotation.strength(first) }.getOrDefault(0) }
    val started = stringResource(R.string.key_rotation_started)
    val cancelled = stringResource(R.string.key_rotation_cancelled)
    fun act(ok: String? = null, block: suspend () -> Unit) {
        busy = true
        scope.launch {
            try {
                block()
                if (ok != null) toast(ok)
            } catch (e: Exception) {
                toast(friendlyError(e))
            } finally {
                busy = false
            }
        }
    }
    Tile(modifier, title = stringResource(R.string.key_rotation_title)) {
        Muted(stringResource(R.string.key_rotation_hint))
        when {
            info.active && !info.byMe -> Text(
                stringResource(
                    if (info.state == "preparing") R.string.key_rotation_other_preparing else R.string.key_rotation_other_switching,
                    info.byName,
                ),
                Modifier.announce(), style = MaterialTheme.typography.bodyMedium, color = T.text,
            )
            info.active -> {
                Text(
                    stringResource(if (info.state == "preparing") R.string.key_rotation_preparing else R.string.key_rotation_switching),
                    Modifier.announce(), style = MaterialTheme.typography.bodyMedium, color = T.text,
                )
                DeviceList(info.devices)
                val s = info.expiresInS
                Muted(
                    when {
                        s >= 2 * 86400 -> pluralStringResource(R.plurals.key_rotation_expires_days, (s / 86400).toInt(), (s / 86400).toInt())
                        s >= 3600 -> pluralStringResource(R.plurals.key_rotation_expires_hours, (s / 3600).toInt(), (s / 3600).toInt())
                        else -> pluralStringResource(R.plurals.key_rotation_expires_minutes, maxOf(1, (s / 60).toInt()), maxOf(1, (s / 60).toInt()))
                    },
                )
                Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    PrimaryButton(
                        stringResource(if (info.state == "preparing") R.string.key_rotation_finish else R.string.key_rotation_stop_waiting),
                        { act { KeyRotation.finish() } }, enabled = !busy,
                    )
                    if (info.state == "preparing") {
                        GhostButton(stringResource(R.string.key_rotation_cancel), { if (!busy) act(cancelled) { KeyRotation.cancel() } })
                    }
                }
                Muted(stringResource(if (info.state == "preparing") R.string.key_rotation_finish_hint else R.string.key_rotation_stop_waiting_hint))
            }
            info.state != "none" -> {
                Text(
                    stringResource(
                        when (info.state) {
                            "done" -> R.string.key_rotation_done
                            "expired" -> R.string.key_rotation_expired
                            else -> R.string.key_rotation_was_cancelled
                        },
                    ),
                    style = MaterialTheme.typography.bodyMedium, color = T.text,
                )
                if (info.byMe) DeviceList(info.devices)
                GhostButton(stringResource(R.string.key_rotation_clear), { act { KeyRotation.dismiss() } }, small = true)
            }
        }
        if (info.active) return@Tile
        if (!open) {
            GhostButton(stringResource(R.string.key_rotation_change), { open = true })
            return@Tile
        }
        Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
            Field(first, { first = it }, stringResource(R.string.key_rotation_new), password = true)
            Field(second, { second = it }, stringResource(R.string.key_rotation_repeat), password = true)
            if (first.isNotEmpty()) {
                val level = stringResource(
                    when (strength) {
                        0 -> R.string.key_rotation_strength0
                        1 -> R.string.key_rotation_strength1
                        2 -> R.string.key_rotation_strength2
                        3 -> R.string.key_rotation_strength3
                        else -> R.string.key_rotation_strength4
                    },
                )
                Muted(stringResource(R.string.key_rotation_strength, level), Modifier.announce())
            }
            if (second.isNotEmpty() && first != second) {
                Text(stringResource(R.string.key_rotation_mismatch), Modifier.announce(), style = MaterialTheme.typography.bodySmall, color = T.warning)
            }
            Muted(stringResource(R.string.key_rotation_form_hint))
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                PrimaryButton(
                    stringResource(R.string.key_rotation_send),
                    {
                        act(started) {
                            KeyRotation.start(first)
                            open = false
                            first = ""
                            second = ""
                        }
                    },
                    enabled = !busy && first.isNotEmpty() && first == second && strength >= 1,
                )
                GhostButton(stringResource(R.string.cancel), { open = false })
            }
        }
    }
}
