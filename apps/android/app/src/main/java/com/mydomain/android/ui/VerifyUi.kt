package com.mydomain.android.ui

import android.content.ActivityNotFoundException
import android.content.Intent
import android.provider.MediaStore
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
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
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import com.mydomain.android.R
import com.mydomain.android.core.Core
import com.mydomain.android.core.KeyVerifyPlan
import com.mydomain.android.core.Verify
import com.mydomain.android.core.friendlyError
import com.mydomain.android.ffi.KeyVerify
import kotlinx.coroutines.launch

/** "Verified", for a device whose security key the user verified. */
@Composable
fun VerifiedBadge(modifier: Modifier = Modifier) {
    val hint = stringResource(R.string.verify_badge_hint)
    Surface(
        modifier.semantics { contentDescription = hint },
        shape = RoundedCornerShape(50),
        color = T.success.copy(alpha = 0.18f),
    ) {
        Text(
            "✓ " + stringResource(R.string.verify_badge),
            Modifier.padding(horizontal = 8.dp, vertical = 2.dp),
            style = MaterialTheme.typography.labelMedium, color = T.text, fontWeight = FontWeight.Bold,
        )
    }
}

/** This phone's own fingerprint (Settings, Add a device). */
@Composable
fun OwnFingerprint() {
    LaunchedEffect(Unit) { if (Verify.ownFingerprint.isEmpty()) runCatching { Verify.loadOwn() } }
    Column(verticalArrangement = Arrangement.spacedBy(2.dp)) {
        Muted(stringResource(R.string.verify_your_key))
        Text(Verify.ownFingerprint.ifEmpty { "…" }, color = T.text, fontFamily = FontFamily.Monospace, style = MaterialTheme.typography.bodyLarge)
        Muted(stringResource(R.string.verify_your_key_hint))
    }
}

/** On a device's screen: the badge and "Verify" (opens [VerifyDialog]). */
@Composable
fun VerifyRow(node: String) {
    val toast = LocalToast.current
    val ctx = LocalContext.current
    val scope = rememberCoroutineScope()
    var info by remember(node) { mutableStateOf<KeyVerify?>(null) }
    var open by remember(node) { mutableStateOf(false) }
    LaunchedEffect(node) { info = runCatching { Verify.load(node) }.getOrNull() }
    Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
        if (Verify.verified[node] == true) VerifiedBadge()
        GhostButton(stringResource(R.string.verify_button), {
            scope.launch {
                try {
                    val v = Verify.load(node)
                    info = v
                    if (v != null) open = true else toast(ctx.getString(R.string.verify_not_yet, Core.peerName(node)))
                } catch (e: Exception) {
                    toast(friendlyError(e))
                }
            }
        }, small = true)
    }
    val v = info
    if (open && v != null) {
        VerifyDialog(v, onChange = { info = it }, close = { open = false })
    }
}

@Composable
private fun VerifyDialog(v: KeyVerify, onChange: (KeyVerify?) -> Unit, close: () -> Unit) {
    val ctx = LocalContext.current
    val toast = LocalToast.current
    val scope = rememberCoroutineScope()
    val name = Core.peerName(v.nodeId).ifBlank { v.name }
    var showCode by remember { mutableStateOf(false) }
    val mark = { on: Boolean ->
        scope.launch {
            try {
                Verify.set(v.nodeId, on)
                toast(ctx.getString(if (on) R.string.verify_done else R.string.verify_undone, name))
                onChange(runCatching { Verify.load(v.nodeId) }.getOrNull())
            } catch (e: Exception) {
                toast(friendlyError(e))
            }
        }
    }
    AlertDialog(
        onDismissRequest = close,
        containerColor = T.bar,
        title = { Text(stringResource(R.string.verify_title, name), color = T.text) },
        text = {
            Column(Modifier.verticalScroll(rememberScrollState()), verticalArrangement = Arrangement.spacedBy(10.dp)) {
                Text(stringResource(R.string.verify_intro, name), color = T.text)
                if (v.keyChanged) Text(stringResource(R.string.verify_key_changed_first, name), color = T.warning)
                Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                    Column(Modifier.weight(1f)) {
                        Muted(stringResource(R.string.verify_this_phone))
                        Text(v.ownFingerprint, color = T.text, fontFamily = FontFamily.Monospace)
                    }
                    Column(Modifier.weight(1f)) {
                        Muted(name)
                        Text(v.theirFingerprint, color = T.text, fontFamily = FontFamily.Monospace)
                    }
                }
                Muted(stringResource(R.string.verify_safety_number))
                Column {
                    KeyVerifyPlan.safetyLines(v.safetyNumber).forEach {
                        Text(it, color = T.text, fontFamily = FontFamily.Monospace, style = MaterialTheme.typography.titleMedium)
                    }
                }
                // The other device scans this phone's code, or this phone
                // scans theirs with the camera app (it opens the link here).
                if (showCode && v.qrSize.toInt() > 0) {
                    val label = stringResource(R.string.verify_qr_label)
                    QrModules(
                        v.qrSize.toInt(), { v.qrModules[it].toInt() != 0 },
                        Modifier.size(220.dp).align(Alignment.CenterHorizontally).semantics { contentDescription = label },
                    )
                } else {
                    GhostButton(stringResource(R.string.verify_show_code), { showCode = true }, small = true)
                }
                GhostButton(stringResource(R.string.verify_scan, name), {
                    try {
                        ctx.startActivity(Intent(MediaStore.INTENT_ACTION_STILL_IMAGE_CAMERA))
                    } catch (_: ActivityNotFoundException) {
                        toast(ctx.getString(R.string.verify_no_camera))
                    }
                }, small = true)
                Muted(stringResource(R.string.verify_scan_hint, name))
                Text(
                    stringResource(if (v.verified) R.string.verify_is_verified else R.string.verify_not_verified, name),
                    color = T.text, fontWeight = FontWeight.Bold,
                )
                if (v.verified) {
                    GhostButton(stringResource(R.string.verify_unmark), { mark(false) }, small = true)
                } else if (!v.keyChanged) {
                    PrimaryButton(stringResource(R.string.verify_match), { mark(true) })
                }
                Muted(stringResource(R.string.verify_mismatch))
            }
        },
        confirmButton = {
            TextButton(close) { Text(stringResource(R.string.verify_close), color = T.text) }
        },
    )
}

/** A verify code scanned with the camera app ([Verify.pendingScan]): the
 *  core checks it, and this says what came of it. */
@Composable
fun VerifyScanResult() {
    val link = Verify.pendingScan ?: return
    var outcome by remember(link) { mutableStateOf<Pair<KeyVerifyPlan.Outcome, String>?>(null) }
    LaunchedEffect(link) {
        outcome = try {
            val r = Verify.scanned(link)
            KeyVerifyPlan.outcome(r.nodeId, r.problem) to (r.nodeId?.let { Core.peerName(it) } ?: "")
        } catch (_: Exception) {
            KeyVerifyPlan.Outcome.NotACode to ""
        }
    }
    val (o, name) = outcome ?: return
    val text = when (o) {
        KeyVerifyPlan.Outcome.Verified -> stringResource(R.string.verify_scan_ok, name)
        KeyVerifyPlan.Outcome.NotACode -> stringResource(R.string.verify_scan_not_a_code)
        KeyVerifyPlan.Outcome.NotForThisDevice -> stringResource(R.string.verify_scan_not_for_this)
        KeyVerifyPlan.Outcome.UnknownDevice -> stringResource(R.string.verify_scan_unknown)
        KeyVerifyPlan.Outcome.KeyChanged -> stringResource(R.string.verify_scan_key_changed)
        KeyVerifyPlan.Outcome.Mismatch -> stringResource(R.string.verify_scan_mismatch)
    }
    val close = { Verify.pendingScan = null }
    AlertDialog(
        onDismissRequest = close,
        containerColor = T.bar,
        title = { Text(stringResource(R.string.verify_scan_title), color = T.text) },
        text = {
            Text(text, color = if (o == KeyVerifyPlan.Outcome.Mismatch) T.danger else T.text)
        },
        confirmButton = {
            TextButton(close) { Text(stringResource(R.string.verify_ok), color = T.text) }
        },
    )
}
