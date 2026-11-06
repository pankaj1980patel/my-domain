package com.mydomain.android.ui

import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import com.mydomain.android.R
import com.mydomain.android.core.KeyChangeInfo
import com.mydomain.android.core.KeyChangePlan
import com.mydomain.android.core.KeyChanges
import com.mydomain.android.core.friendlyError
import kotlinx.coroutines.launch

/**
 * "<name>'s security key changed": on the device's screen (until the user
 * decides or says "Not now") and in the connection map's details (always,
 * while it's refused, [showDismissed]). "Trust the new key" asks first,
 * showing the old and new keys' short fingerprints; only the key shown is
 * trusted.
 */
@Composable
fun KeyChangeBanner(node: String, showDismissed: Boolean = false) {
    LaunchedEffect(node) { runCatching { KeyChanges.load() } }
    val c = KeyChangePlan.banner(KeyChanges.list, node, showDismissed) ?: return
    val toast = LocalToast.current
    val scope = rememberCoroutineScope()
    var asking by remember(node) { mutableStateOf(false) }
    val name = KeyChanges.nameOf(c)
    Surface(
        Modifier.fillMaxWidth().semantics { liveRegion = LiveRegionMode.Polite },
        shape = MaterialTheme.shapes.medium,
        color = T.warning.copy(alpha = 0.12f),
        border = BorderStroke(1.dp, T.warning),
    ) {
        Column(Modifier.padding(14.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            Text(stringResource(R.string.key_change_title, name), style = MaterialTheme.typography.titleSmall, color = T.text, fontWeight = FontWeight.Bold)
            Text(stringResource(R.string.key_change_body), style = MaterialTheme.typography.bodyMedium, color = T.text)
            Muted(stringResource(if (c.dismissed) R.string.key_change_dismissed else R.string.key_change_refused, name))
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                SoftButton(stringResource(R.string.key_change_trust), { asking = true })
                if (!c.dismissed) {
                    GhostButton(stringResource(R.string.key_change_not_now), {
                        scope.launch { try { KeyChanges.dismiss(node) } catch (e: Exception) { toast(friendlyError(e)) } }
                    })
                }
            }
        }
    }
    if (asking) {
        val trusted = stringResource(R.string.key_change_trusted, name)
        TrustDialog(c, name, close = { asking = false }) {
            asking = false
            scope.launch {
                try {
                    KeyChanges.trust(c)
                    toast(trusted)
                } catch (e: Exception) {
                    toast(friendlyError(e))
                }
            }
        }
    }
}

@Composable
private fun TrustDialog(c: KeyChangeInfo, name: String, close: () -> Unit, trust: () -> Unit) {
    AlertDialog(
        onDismissRequest = close,
        containerColor = T.bar,
        title = { Text(stringResource(R.string.key_change_confirm_title, name), color = T.text) },
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                Text(stringResource(R.string.key_change_body), color = T.text)
                Text(stringResource(R.string.key_change_confirm_body, name), color = T.text)
                Muted(stringResource(R.string.key_change_old))
                Text(c.oldFingerprint, color = T.text, fontFamily = FontFamily.Monospace)
                Muted(stringResource(R.string.key_change_new))
                Text(c.newFingerprint, color = T.text, fontFamily = FontFamily.Monospace)
            }
        },
        // Cancel is the easy way out; trusting is never the default.
        dismissButton = {
            TextButton(close) { Text(stringResource(R.string.cancel), color = T.text2) }
        },
        confirmButton = {
            TextButton(trust) { Text(stringResource(R.string.key_change_trust), color = T.warning) }
        },
    )
}
