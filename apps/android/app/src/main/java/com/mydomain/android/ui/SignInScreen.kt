package com.mydomain.android.ui

import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.heading
import android.content.SharedPreferences
import androidx.compose.foundation.Image
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
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
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import com.mydomain.android.R
import com.mydomain.android.core.Core
import com.mydomain.android.core.JoinLinks
import com.mydomain.android.core.friendlyError
import com.mydomain.android.ffi.generateKey
import kotlinx.coroutines.launch

const val DEFAULT_SERVER = "https://mydomain.servehttp.com"

/** Welcome / sign in. The server is pre-filled and tucked under "Advanced":
 *  nobody should have to type a URL. "Join with a code" signs in with the
 *  6-digit code a signed-in device shows under Add a device; a scanned
 *  join QR code ([Core.pendingJoin]) is asked about, then joined. */
@Composable
fun SignInScreen(prefs: SharedPreferences) {
    var create by remember { mutableStateOf(false) }
    var server by remember { mutableStateOf(prefs.getString("server", null) ?: DEFAULT_SERVER) }
    var user by remember { mutableStateOf(prefs.getString("user", "") ?: "") }
    var pass by remember { mutableStateOf("") }
    var key by remember { mutableStateOf("") }
    var advanced by remember { mutableStateOf(false) }
    var busy by remember { mutableStateOf(false) }
    var error by remember { mutableStateOf("") }
    var join by remember { mutableStateOf(false) }
    var code by remember { mutableStateOf("") }
    val scope = rememberCoroutineScope()
    val missing = stringResource(R.string.signin_missing_fields)
    val missingCode = stringResource(R.string.signin_join_missing)
    val badLink = stringResource(R.string.error_bad_join_link)
    // A scanned QR code: asked about below before it signs this phone in.
    val scanned = Core.pendingJoin
    val scannedLink = scanned?.let { JoinLinks.parse(it) }
    LaunchedEffect(scanned) {
        if (scanned != null && scannedLink == null) {
            error = badLink
            Core.pendingJoin = null
        }
    }

    fun joinWith(codeOrLink: String) {
        busy = true; error = ""
        scope.launch {
            try {
                val url = server.trim().ifEmpty { DEFAULT_SERVER }
                Core.joinAccount(url, codeOrLink)
                val s = Core.session
                prefs.edit().putString("server", s?.serverUrl ?: url).putString("user", s?.username ?: "").apply()
            } catch (e: Exception) {
                error = friendlyError(e)
            } finally {
                busy = false
                Core.pendingJoin = null
            }
        }
    }

    fun submitJoin() {
        val c = code.trim()
        if (!JoinLinks.isJoinLink(c) && JoinLinks.codeDigits(c) == null) {
            error = missingCode; return
        }
        joinWith(c)
    }

    fun submit() {
        if (join) return submitJoin()
        if (user.isBlank() || pass.isEmpty() || key.isBlank()) {
            error = missing; return
        }
        busy = true; error = ""
        scope.launch {
            try {
                val url = server.trim().ifEmpty { DEFAULT_SERVER }
                Core.signIn(url, user.trim(), pass, key.trim(), create)
                prefs.edit().putString("server", url).putString("user", user.trim()).apply()
            } catch (e: Exception) {
                error = friendlyError(e)
            } finally {
                busy = false
            }
        }
    }

    Column(
        Modifier.fillMaxSize().verticalScroll(rememberScrollState()).imePadding().padding(24.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        Column(Modifier.widthIn(max = 420.dp).fillMaxWidth(), verticalArrangement = Arrangement.spacedBy(12.dp)) {
            Spacer(Modifier.height(16.dp))
            // Dark themes: the soft-edged mark, whose own sky fades into the
            // background (nudged left so the galaxy lines up with the text).
            // Cosmic Light: the frameless light galaxy.
            Image(
                painterResource(logoRes()), contentDescription = null,
                modifier = Modifier.size(112.dp).offset(x = if (T.isLight) 0.dp else (-14).dp),
            )
            Wordmark()
            Tagline()
            Text(
                stringResource(
                    when {
                        join || scannedLink != null -> R.string.signin_join_title
                        create -> R.string.signin_title_create
                        else -> R.string.signin_title_welcome
                    },
                ),
                Modifier.semantics { heading() }, style = MaterialTheme.typography.headlineSmall, color = T.text,
            )

            if (scannedLink != null) {
                // A scanned join QR code: say whose account, and ask.
                Text(boldArg(R.string.signin_join_confirm, scannedLink.username), style = MaterialTheme.typography.bodyMedium, color = T.text)
                Muted(scannedLink.server)
                if (error.isNotEmpty()) ErrorText(error)
                PrimaryButton(
                    stringResource(if (busy) R.string.signin_joining else R.string.signin_join),
                    { scanned?.let { joinWith(it) } }, Modifier.fillMaxWidth(), enabled = !busy,
                )
                TextButton(onClick = { Core.pendingJoin = null; error = "" }, enabled = !busy) {
                    Text(stringResource(R.string.join_close), color = T.text2)
                }
            } else {
                Muted(stringResource(if (join) R.string.signin_join_hint else R.string.signin_tagline))
                Spacer(Modifier.height(4.dp))

                if (join) {
                    Field(code, { code = it }, stringResource(R.string.signin_join_code))
                } else {
                    Field(user, { user = it }, stringResource(R.string.signin_username))
                    Field(pass, { pass = it }, stringResource(R.string.signin_password), password = true)
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        Field(key, { key = it }, stringResource(R.string.signin_device_key), Modifier.weight(1f), password = true)
                        if (create) {
                            GhostButton(stringResource(R.string.signin_generate), { key = generateKey() }, Modifier.padding(start = 8.dp), small = true)
                        }
                    }
                    Muted(stringResource(R.string.signin_key_hint))
                }

                TextButton(onClick = { advanced = !advanced }) {
                    Text(stringResource(if (advanced) R.string.signin_hide_advanced else R.string.signin_advanced), color = T.text2)
                }
                if (advanced) Field(server, { server = it }, stringResource(R.string.signin_server))

                if (error.isNotEmpty()) ErrorText(error)

                PrimaryButton(
                    stringResource(
                        when {
                            join -> if (busy) R.string.signin_joining else R.string.signin_join
                            busy -> R.string.signin_busy
                            create -> R.string.signin_create_account
                            else -> R.string.signin_sign_in
                        },
                    ),
                    ::submit, Modifier.fillMaxWidth(), enabled = !busy,
                )
                TextButton(onClick = { join = !join; error = "" }, Modifier.fillMaxWidth()) {
                    Text(stringResource(if (join) R.string.signin_use_password else R.string.signin_join_with_code), color = T.text)
                }
                if (!join) Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.Center, verticalAlignment = Alignment.CenterVertically) {
                    Muted(stringResource(if (create) R.string.signin_have_account else R.string.signin_new_here))
                    TextButton(onClick = { create = !create; error = "" }) {
                        Text(stringResource(if (create) R.string.signin_sign_in else R.string.signin_create_an_account), color = T.text)
                    }
                }
            }
        }
    }
}
