package com.mydomain.android.ui

import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.heading
import android.content.ClipboardManager
import android.content.SharedPreferences
import androidx.annotation.DrawableRes
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
import androidx.compose.foundation.layout.width
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
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import com.mydomain.android.R
import com.mydomain.android.core.ClipPrivacy
import com.mydomain.android.core.Core
import com.mydomain.android.core.JoinLinks
import com.mydomain.android.core.friendlyError
import com.mydomain.android.ffi.generateKey
import kotlinx.coroutines.launch

const val DEFAULT_SERVER = "https://mydomain.servehttp.com"

/** How the sign-in screen is being used: the two big choices first. */
private enum class SignInMode { Choose, Join, Create, Password }

/** The device key made for a new account, shown once over the app
 *  ([NewAccountKeyDialog]); in memory only, never written anywhere. */
object NewAccountKey {
    var shown by mutableStateOf<String?>(null)
}

/** Welcome / sign in. Two big choices: "I already use it on another
 *  device" (the 6-digit code or QR code a signed-in device shows under Add
 *  a device: no password, and the device key comes with it) and "This is
 *  my first device" (a new account; a strong device key is made for it and
 *  shown once). "Sign in with a password" is the smaller third way, for an
 *  account you have. The server is pre-filled and tucked under "Advanced".
 *  A scanned join QR code ([Core.pendingJoin]) is asked about, then joined.
 *  "Welcome back" only on a phone that was signed in before. */
@Composable
fun SignInScreen(prefs: SharedPreferences) {
    val before = !prefs.getString("user", null).isNullOrBlank()
    // Typed text and the mode survive a rotation.
    var mode by rememberSaveable { mutableStateOf(SignInMode.Choose) }
    var server by rememberSaveable { mutableStateOf(prefs.getString("server", null) ?: DEFAULT_SERVER) }
    var user by rememberSaveable { mutableStateOf(prefs.getString("user", "") ?: "") }
    var pass by rememberSaveable { mutableStateOf("") }
    var key by rememberSaveable { mutableStateOf("") }
    var advanced by rememberSaveable { mutableStateOf(false) }
    // Not kept over a rotation: the sign-in under way is this screen's.
    var busy by remember { mutableStateOf(false) }
    var error by rememberSaveable { mutableStateOf("") }
    var code by rememberSaveable { mutableStateOf("") }
    val scope = rememberCoroutineScope()
    val missing = stringResource(R.string.signin_missing_fields)
    val missingNew = stringResource(R.string.signin_missing_user_pass)
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
    val go = { m: SignInMode -> mode = m; error = "" }

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
        if (mode == SignInMode.Join) return submitJoin()
        val create = mode == SignInMode.Create
        if (user.isBlank() || pass.isEmpty() || (!create && key.isBlank())) {
            error = if (create) missingNew else missing; return
        }
        // A new account: a strong key made here unless one was typed under
        // Advanced (192 random bits; stronger than any typed one). The same
        // key encrypts the same way however it was made.
        val made = create && key.isBlank()
        val deviceKey = if (made) generateKey() else key.trim()
        busy = true; error = ""
        scope.launch {
            try {
                val url = server.trim().ifEmpty { DEFAULT_SERVER }
                // Shown as soon as the app opens (before the sign-in returns,
                // the app may already be on screen).
                if (made) NewAccountKey.shown = deviceKey
                Core.signIn(url, user.trim(), pass, deviceKey, create)
                prefs.edit().putString("server", url).putString("user", user.trim()).apply()
            } catch (e: Exception) {
                if (made) NewAccountKey.shown = null
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
                        mode == SignInMode.Join || scannedLink != null -> R.string.signin_join_title
                        mode == SignInMode.Create -> R.string.signin_title_create
                        mode == SignInMode.Password -> R.string.signin_sign_in
                        before -> R.string.signin_title_welcome
                        else -> R.string.signin_title_start
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
            } else if (mode == SignInMode.Choose) {
                Muted(stringResource(R.string.signin_tagline))
                ChoiceCard(
                    R.drawable.ic_lucide_monitor_smartphone, Hues.files,
                    stringResource(R.string.signin_choice_join_title), stringResource(R.string.signin_choice_join_body),
                ) { go(SignInMode.Join) }
                ChoiceCard(
                    R.drawable.ic_lucide_user, Hues.messages,
                    stringResource(R.string.signin_choice_new_title), stringResource(R.string.signin_choice_new_body),
                ) { go(SignInMode.Create) }
                TextButton(onClick = { go(SignInMode.Password) }, Modifier.fillMaxWidth()) {
                    Text(stringResource(R.string.signin_choice_password), color = T.text)
                }
                if (error.isNotEmpty()) ErrorText(error)
            } else {
                Muted(
                    stringResource(
                        when (mode) {
                            SignInMode.Join -> R.string.signin_join_hint
                            SignInMode.Create -> R.string.signin_create_hint
                            else -> R.string.signin_tagline
                        },
                    ),
                )
                Spacer(Modifier.height(4.dp))

                when (mode) {
                    SignInMode.Join -> Field(code, { code = it }, stringResource(R.string.signin_join_code))
                    else -> {
                        Field(user, { user = it }, stringResource(R.string.signin_username))
                        Field(pass, { pass = it }, stringResource(R.string.signin_password), password = true)
                        if (mode == SignInMode.Password) {
                            Field(key, { key = it }, stringResource(R.string.signin_device_key), password = true)
                            Muted(stringResource(R.string.signin_key_hint))
                        }
                    }
                }

                TextButton(onClick = { advanced = !advanced }) {
                    Text(stringResource(if (advanced) R.string.signin_hide_advanced else R.string.signin_advanced), color = T.text2)
                }
                if (advanced) {
                    Field(server, { server = it }, stringResource(R.string.signin_server))
                    // A new account may bring its own key (blank: one is made).
                    if (mode == SignInMode.Create) {
                        Field(key, { key = it }, stringResource(R.string.signin_own_key), password = true)
                        Muted(stringResource(R.string.signin_own_key_hint))
                    }
                }

                if (error.isNotEmpty()) ErrorText(error)

                PrimaryButton(
                    stringResource(
                        when {
                            mode == SignInMode.Join -> if (busy) R.string.signin_joining else R.string.signin_join
                            busy -> R.string.signin_busy
                            mode == SignInMode.Create -> R.string.signin_create_account
                            else -> R.string.signin_sign_in
                        },
                    ),
                    ::submit, Modifier.fillMaxWidth(), enabled = !busy,
                )
                // Back to the two choices.
                TextButton(onClick = { go(SignInMode.Choose) }, Modifier.fillMaxWidth(), enabled = !busy) {
                    Text(stringResource(R.string.signin_other_ways), color = T.text)
                }
            }
        }
    }
}

/** One of the sign-in screen's big choices: an icon, what it is, and what
 *  happens next; the whole card is the button. */
@Composable
private fun ChoiceCard(@DrawableRes icon: Int, h: Hue, title: String, body: String, onClick: () -> Unit) {
    SoftCard(onClick = onClick) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            IconBubble(icon, h, 52.dp)
            Spacer(Modifier.width(14.dp))
            Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(2.dp)) {
                Text(title, style = MaterialTheme.typography.titleMedium, color = T.text, fontWeight = FontWeight.Bold)
                Muted(body)
            }
            Chevron()
        }
    }
}

/** A new account's device key, once: what it's for, Copy, and "I've saved
 *  it". It isn't kept as typed anywhere (this phone keeps the encryption
 *  key made from it), so it can't be shown again. */
@Composable
fun NewAccountKeyDialog() {
    val key = NewAccountKey.shown ?: return
    val ctx = LocalContext.current
    val toast = LocalToast.current
    val copied = stringResource(R.string.newkey_copied)
    AlertDialog(
        // Only the button closes it: a stray tap mustn't lose the key.
        onDismissRequest = {},
        containerColor = T.bar,
        title = { Text(stringResource(R.string.newkey_title), color = T.text) },
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
                Text(stringResource(R.string.newkey_body), color = T.text2, style = MaterialTheme.typography.bodyMedium)
                Text(
                    key, color = T.text, fontFamily = FontFamily.Monospace,
                    style = MaterialTheme.typography.bodyLarge, fontWeight = FontWeight.SemiBold,
                )
                SoftButton(stringResource(R.string.newkey_copy), {
                    // Marked sensitive: Android hides its preview (and, with
                    // "Never sync passwords" on, it isn't synced).
                    runCatching {
                        ctx.getSystemService(ClipboardManager::class.java)
                            ?.setPrimaryClip(ClipPrivacy.clipOf("my-domain", key, sensitive = true))
                    }.onSuccess { toast(copied) }
                }, Modifier.fillMaxWidth())
            }
        },
        confirmButton = {
            TextButton({ NewAccountKey.shown = null }) { Text(stringResource(R.string.newkey_done), color = T.accentFg) }
        },
    )
}
