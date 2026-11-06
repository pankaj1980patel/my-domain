package com.mydomain.android.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.unit.dp
import com.mydomain.android.R
import com.mydomain.android.core.RemovedDevices
import com.mydomain.android.core.friendlyError
import kotlinx.coroutines.launch

/**
 * "Remove device" on a device's screen: asks first, saying it's for good
 * (for that install: your other devices refuse it too, even with the
 * passphrase) and that "Allow again" in Settings takes it back.
 */
@Composable
fun RemoveDeviceRow(node: String, name: String, onRemoved: () -> Unit) {
    val toast = LocalToast.current
    val scope = rememberCoroutineScope()
    var asking by remember(node) { mutableStateOf(false) }
    val removed = stringResource(R.string.remove_device_done, name)
    GhostButton(stringResource(R.string.remove_device), { asking = true }, small = true)
    if (asking) {
        AlertDialog(
            onDismissRequest = { asking = false },
            containerColor = T.bar,
            title = { Text(stringResource(R.string.remove_device_title, name), color = T.text) },
            text = {
                Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                    Text(stringResource(R.string.remove_device_body, name), color = T.text)
                    Muted(stringResource(R.string.remove_device_passphrase))
                }
            },
            // Cancel is the easy way out; removing is never the default.
            dismissButton = {
                TextButton({ asking = false }) { Text(stringResource(R.string.cancel), color = T.text2) }
            },
            confirmButton = {
                TextButton({
                    asking = false
                    scope.launch {
                        try {
                            RemovedDevices.remove(node)
                            toast(removed)
                            onRemoved()
                        } catch (e: Exception) {
                            toast(friendlyError(e))
                        }
                    }
                }) { Text(stringResource(R.string.remove_device), color = T.warning) }
            },
        )
    }
}

/** Settings: the removed devices, each with "Allow again". */
@Composable
fun RemovedDevicesTile(modifier: Modifier = Modifier) {
    LaunchedEffect(Unit) { runCatching { RemovedDevices.load() } }
    val toast = LocalToast.current
    val scope = rememberCoroutineScope()
    Tile(modifier, title = stringResource(R.string.removed_devices_title)) {
        val list = RemovedDevices.list
        if (list.isEmpty()) {
            Muted(stringResource(R.string.removed_devices_none))
            return@Tile
        }
        Muted(stringResource(R.string.removed_devices_hint))
        for (d in list) {
            val allowed = stringResource(R.string.removed_devices_allowed, d.name)
            Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(2.dp)) {
                    Text(d.name, style = MaterialTheme.typography.bodyLarge, color = T.text)
                    if (d.fingerprint.isNotEmpty()) {
                        Text(d.fingerprint, style = MaterialTheme.typography.bodySmall, color = T.text2, fontFamily = FontFamily.Monospace)
                    }
                    Muted(
                        if (d.byMe) stringResource(R.string.removed_devices_by_me)
                        else stringResource(R.string.removed_devices_by, d.byName),
                    )
                }
                GhostButton(stringResource(R.string.removed_devices_allow), {
                    scope.launch {
                        try {
                            RemovedDevices.allow(d.nodeId)
                            toast(allowed)
                        } catch (e: Exception) {
                            toast(friendlyError(e))
                        }
                    }
                }, small = true)
            }
        }
    }
}
