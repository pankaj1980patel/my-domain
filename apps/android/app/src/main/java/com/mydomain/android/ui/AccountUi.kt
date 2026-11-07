package com.mydomain.android.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
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
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.res.pluralStringResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import com.mydomain.android.R
import com.mydomain.android.core.Account
import com.mydomain.android.core.AccountPlan
import com.mydomain.android.core.AccountSessionInfo
import com.mydomain.android.core.Ago
import com.mydomain.android.core.Core
import com.mydomain.android.core.KeyRotation
import com.mydomain.android.core.PasswordFormError
import com.mydomain.android.core.friendlyError
import com.mydomain.android.ffi.MdException
import kotlinx.coroutines.launch

/** "5 min ago" for a time in seconds since the epoch. */
@Composable
private fun agoText(atS: Long): String {
    val now = System.currentTimeMillis() / 1000
    return when (val a = AccountPlan.ago(atS, now)) {
        is Ago.JustNow -> stringResource(R.string.account_ago_now)
        is Ago.Minutes -> pluralStringResource(R.plurals.account_ago_minutes, a.n, a.n)
        is Ago.Hours -> pluralStringResource(R.plurals.account_ago_hours, a.n, a.n)
        is Ago.Days -> pluralStringResource(R.plurals.account_ago_days, a.n, a.n)
    }
}

/**
 * Settings → Account: change the account password (not the encryption
 * passphrase) and sign out all devices. Placed inside the Account tile.
 */
@Composable
fun AccountSecurity() {
    var changing by remember { mutableStateOf(false) }
    var signingOutAll by remember { mutableStateOf(false) }
    Muted(stringResource(R.string.account_password_hint))
    Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
        GhostButton(stringResource(R.string.account_change_password), { changing = true })
        GhostButton(stringResource(R.string.account_sign_out_all), { signingOutAll = true })
    }
    if (changing) ChangePasswordDialog { changing = false }
    if (signingOutAll) SignOutAllDialog { signingOutAll = false }
}

@Composable
private fun formErrorText(e: PasswordFormError): String = stringResource(
    when (e) {
        PasswordFormError.NoCurrent -> R.string.account_password_need_current
        PasswordFormError.TooShort -> R.string.account_password_too_short
        PasswordFormError.Mismatch -> R.string.account_password_mismatch
        PasswordFormError.SameAsCurrent -> R.string.account_password_same
    },
)

/** Current password, the new one twice with its strength, and what will
 *  happen to the other devices, said before the button. */
@Composable
private fun ChangePasswordDialog(close: () -> Unit) {
    val toast = LocalToast.current
    val scope = rememberCoroutineScope()
    var current by remember { mutableStateOf("") }
    var first by remember { mutableStateOf("") }
    var second by remember { mutableStateOf("") }
    var strength by remember { mutableIntStateOf(0) }
    var busy by remember { mutableStateOf(false) }
    var error by remember { mutableStateOf("") }
    LaunchedEffect(first) { strength = runCatching { KeyRotation.strength(first) }.getOrDefault(0) }
    val done = stringResource(R.string.account_password_changed)
    val formError = AccountPlan.check(current, first, second)
    AlertDialog(
        onDismissRequest = { if (!busy) close() },
        containerColor = T.bar,
        title = { Text(stringResource(R.string.account_change_password), color = T.text) },
        text = {
            Column(Modifier.verticalScroll(rememberScrollState()), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                Muted(stringResource(R.string.account_password_hint))
                Field(current, { current = it }, stringResource(R.string.account_password_current), password = true)
                Field(first, { first = it }, stringResource(R.string.account_password_new), password = true)
                Field(second, { second = it }, stringResource(R.string.account_password_repeat), password = true)
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
                if (formError != null && first.isNotEmpty() && (second.isNotEmpty() || formError != PasswordFormError.Mismatch)) {
                    Text(formErrorText(formError), Modifier.announce(), style = MaterialTheme.typography.bodySmall, color = T.warning)
                }
                Text(
                    stringResource(R.string.account_password_warning),
                    style = MaterialTheme.typography.bodyMedium, color = T.text,
                )
                if (error.isNotEmpty()) ErrorText(error)
            }
        },
        confirmButton = {
            PrimaryButton(
                stringResource(if (busy) R.string.account_working else R.string.account_password_confirm),
                {
                    busy = true; error = ""
                    scope.launch {
                        try {
                            Account.changePassword(current, first)
                            toast(done)
                            close()
                        } catch (e: Exception) {
                            error = friendlyError(e)
                        } finally {
                            busy = false
                        }
                    }
                },
                enabled = !busy && formError == null,
            )
        },
        dismissButton = {
            TextButton(onClick = close, enabled = !busy) { Text(stringResource(R.string.cancel), color = T.text2) }
        },
    )
}

/** Confirm signing out every device; the password (optional) signs this
 *  phone straight back in. */
@Composable
private fun SignOutAllDialog(close: () -> Unit) {
    val toast = LocalToast.current
    val scope = rememberCoroutineScope()
    var pass by remember { mutableStateOf("") }
    var busy by remember { mutableStateOf(false) }
    var error by remember { mutableStateOf("") }
    val back = stringResource(R.string.account_sign_out_all_done)
    AlertDialog(
        onDismissRequest = { if (!busy) close() },
        containerColor = T.bar,
        title = { Text(stringResource(R.string.account_sign_out_all_title), color = T.text) },
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                Text(stringResource(R.string.account_sign_out_all_body), style = MaterialTheme.typography.bodyMedium, color = T.text)
                Field(pass, { pass = it }, stringResource(R.string.account_sign_out_all_password), password = true)
                Muted(stringResource(R.string.account_sign_out_all_password_hint))
                if (error.isNotEmpty()) ErrorText(error)
            }
        },
        confirmButton = {
            PrimaryButton(
                stringResource(if (busy) R.string.account_working else R.string.account_sign_out_all),
                {
                    busy = true; error = ""
                    scope.launch {
                        try {
                            // Signed back in: say so. Not: sign-in shows by itself.
                            if (Account.signOutAll(pass)) toast(back)
                            close()
                        } catch (e: Exception) {
                            error = friendlyError(e)
                        } finally {
                            busy = false
                        }
                    }
                },
                enabled = !busy,
            )
        },
        dismissButton = {
            TextButton(onClick = close, enabled = !busy) { Text(stringResource(R.string.cancel), color = T.text2) }
        },
    )
}

/** Settings: the account's signed-in sessions, each but this phone's with
 *  "Sign out". */
@Composable
fun SessionsTile(modifier: Modifier = Modifier) {
    LaunchedEffect(Unit) { Account.loadSessions() }
    val toast = LocalToast.current
    val scope = rememberCoroutineScope()
    var confirm by remember { mutableStateOf<AccountSessionInfo?>(null) }
    val signedOutOne = stringResource(R.string.account_session_signed_out)
    Tile(modifier, title = stringResource(R.string.account_sessions_title)) {
        Muted(stringResource(R.string.account_sessions_hint))
        Account.sessionsError?.let { Muted(it, Modifier.announce()) }
        for (s in Account.sessions) {
            val name = AccountPlan.displayName(s) { node -> Core.peerName(node) }
                ?: stringResource(R.string.account_session_unknown)
            Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                Column(Modifier.weight(1f)) {
                    Text(
                        if (s.current) stringResource(R.string.account_session_this, name) else name,
                        style = MaterialTheme.typography.bodyLarge, color = T.text,
                    )
                    Muted(stringResource(R.string.account_session_times, agoText(s.created), agoText(s.lastSeen)))
                }
                if (!s.current) GhostButton(stringResource(R.string.settings_sign_out), { confirm = s }, small = true)
            }
        }
        GhostButton(stringResource(R.string.account_sessions_refresh), { scope.launch { Account.loadSessions() } }, small = true)
    }
    val c = confirm
    if (c != null) {
        val name = AccountPlan.displayName(c) { node -> Core.peerName(node) } ?: stringResource(R.string.account_session_unknown)
        AlertDialog(
            onDismissRequest = { confirm = null },
            containerColor = T.bar,
            title = { Text(stringResource(R.string.account_session_sign_out_title, name), color = T.text) },
            text = { Text(stringResource(R.string.account_session_sign_out_body), color = T.text) },
            confirmButton = {
                PrimaryButton(stringResource(R.string.settings_sign_out), {
                    confirm = null
                    scope.launch {
                        try {
                            Account.signOutSession(c.id)
                            toast(signedOutOne)
                        } catch (e: Exception) {
                            toast(friendlyError(e))
                        }
                    }
                })
            },
            dismissButton = {
                TextButton(onClick = { confirm = null }) { Text(stringResource(R.string.cancel), color = T.text2) }
            },
        )
    }
}

/** Shown instead of the app when the server signed this phone out: the
 *  password alone signs back in (the encryption key is still here). */
@Composable
fun SignedOutScreen(username: String) {
    val scope = rememberCoroutineScope()
    var pass by remember { mutableStateOf("") }
    var busy by remember { mutableStateOf(false) }
    var error by remember { mutableStateOf("") }
    fun startOver() {
        scope.launch { runCatching { Account.startOver() } }
    }
    Column(
        Modifier.fillMaxSize().verticalScroll(rememberScrollState()).imePadding().padding(24.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        Column(Modifier.widthIn(max = 420.dp).fillMaxWidth(), verticalArrangement = Arrangement.spacedBy(12.dp)) {
            Spacer(Modifier.height(32.dp))
            Text(
                stringResource(R.string.account_signed_out_title),
                Modifier.semantics { heading() }, style = MaterialTheme.typography.headlineSmall, color = T.text,
            )
            Text(
                stringResource(R.string.account_signed_out_body, username),
                Modifier.announce(urgent = true), style = MaterialTheme.typography.bodyMedium, color = T.text,
            )
            Field(pass, { pass = it }, stringResource(R.string.signin_password), password = true)
            if (error.isNotEmpty()) ErrorText(error)
            PrimaryButton(
                stringResource(if (busy) R.string.signin_busy else R.string.account_sign_in_again),
                {
                    busy = true; error = ""
                    scope.launch {
                        try {
                            Account.signInAgain(pass)
                        } catch (e: MdException.NoKey) {
                            // The key is gone too: the full sign-in.
                            startOver()
                        } catch (e: Exception) {
                            error = friendlyError(e)
                        } finally {
                            busy = false
                        }
                    }
                },
                Modifier.fillMaxWidth(), enabled = !busy && pass.isNotEmpty(),
            )
            TextButton(onClick = ::startOver, Modifier.fillMaxWidth(), enabled = !busy) {
                Text(stringResource(R.string.account_sign_in_another_way), color = T.text)
            }
        }
    }
}
