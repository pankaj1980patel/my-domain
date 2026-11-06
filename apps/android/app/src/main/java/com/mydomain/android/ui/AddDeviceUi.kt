package com.mydomain.android.ui

import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.size
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.mydomain.android.R
import com.mydomain.android.core.Core
import com.mydomain.android.core.JoinLinks
import com.mydomain.android.core.friendlyError
import com.mydomain.android.core.niceName
import com.mydomain.android.ffi.JoinInvite
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch

/**
 * "Add a device" (Settings → Account): a one-time QR code of a join link,
 * and the same invite as a 6-digit code, valid 5 minutes (mdcore `join`).
 * Watches for the new device; when it joins, says so and closes. Closing it
 * (or leaving the screen) withdraws the invite.
 */
@Composable
fun AddDevice() {
    var open by remember { mutableStateOf(false) }
    // Bumped for a new code.
    var round by remember { mutableIntStateOf(0) }
    if (!open) {
        GhostButton(stringResource(R.string.join_add_device), { open = true; round++ })
    } else {
        InvitePanel(round, onNewCode = { round++ }, onClose = { open = false })
    }
}

@Composable
private fun InvitePanel(round: Int, onNewCode: () -> Unit, onClose: () -> Unit) {
    val ctx = LocalContext.current
    val toast = LocalToast.current
    val scope = rememberCoroutineScope()
    var invite by remember { mutableStateOf<JoinInvite?>(null) }
    var left by remember { mutableIntStateOf(0) }
    var error by remember { mutableStateOf("") }
    // Still claimable: withdrawn when the panel goes away.
    var live by remember { mutableStateOf(false) }

    LaunchedEffect(round) {
        invite = null; error = ""; live = false
        try {
            val inv = Core.joinInvite()
            invite = inv
            left = inv.expiresIn.toInt()
            live = true
            var ticks = 0
            while (left > 0) {
                delay(1000)
                left -= 1
                ticks += 1
                if (ticks % 2 != 0) continue
                val s = runCatching { Core.joinStatus() }.getOrNull() ?: continue
                if (s.state == "joined") {
                    live = false
                    toast(ctx.getString(R.string.join_joined, niceName(s.name ?: "")))
                    runCatching { Core.refreshDevices() }
                    onClose()
                    break
                } else if (s.state == "none") {
                    left = 0
                }
            }
            if (left <= 0) live = false
        } catch (e: Exception) {
            error = friendlyError(e)
        }
    }

    DisposableEffect(Unit) {
        onDispose {
            if (live) Thread { runCatching { Core.engine()?.joinCancel() } }.start()
        }
    }

    Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
        val inv = invite
        when {
            error.isNotEmpty() -> ErrorText(error)
            inv == null -> Muted(stringResource(R.string.join_making))
            left <= 0 -> {
                Text(stringResource(R.string.join_expired), style = MaterialTheme.typography.bodyMedium, color = T.text)
                PrimaryButton(stringResource(R.string.join_new_code), onNewCode)
            }
            else -> {
                val label = stringResource(R.string.join_qr_label)
                QrImage(inv, Modifier.size(220.dp).semantics { contentDescription = label })
                Muted(stringResource(R.string.join_scan))
                Text(
                    JoinLinks.spaced(inv.code),
                    style = MaterialTheme.typography.headlineMedium.copy(fontFamily = FontFamily.Monospace, letterSpacing = 4.sp),
                    fontWeight = FontWeight.Bold, color = T.text,
                )
                Muted(stringResource(R.string.join_expires_in, JoinLinks.mmss(left)))
            }
        }
        Muted(stringResource(R.string.join_privacy))
        GhostButton(stringResource(R.string.join_close), {
            if (live) scope.launch { runCatching { Core.joinCancel() } }
            live = false
            onClose()
        }, small = true)
    }
}

/** The QR code, black on white whatever the theme (scanners need the
 *  contrast), with the standard 4-module quiet zone. */
@Composable
private fun QrImage(inv: JoinInvite, modifier: Modifier) {
    val n = inv.qrSize.toInt()
    Canvas(modifier.background(Color.White)) {
        val cell = size.minDimension / (n + 8)
        for (y in 0 until n) {
            for (x in 0 until n) {
                if (inv.qrModules[y * n + x].toInt() != 0) {
                    drawRect(Color.Black, topLeft = Offset((x + 4) * cell, (y + 4) * cell), size = Size(cell + 0.5f, cell + 0.5f))
                }
            }
        }
    }
}
