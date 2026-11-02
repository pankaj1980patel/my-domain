package com.mydomain.android.ui

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
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.unit.dp
import com.mydomain.android.R
import com.mydomain.android.core.Core
import com.mydomain.android.core.friendlyError
import com.mydomain.android.ffi.generateKey
import kotlinx.coroutines.launch

const val DEFAULT_SERVER = "https://152-67-7-50.sslip.io"

/** Welcome / sign in. The server is pre-filled and tucked under "Advanced":
 *  nobody should have to type a URL. */
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
    val scope = rememberCoroutineScope()

    fun submit() {
        if (user.isBlank() || pass.isEmpty() || key.isBlank()) {
            error = "Fill in your username, password and device key."; return
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
            // Soft-edged mark: the logo's own sky fades into the background.
            Image(
                painterResource(R.drawable.logo_mark), contentDescription = null,
                modifier = Modifier.size(112.dp).offset(x = (-14).dp),
            )
            Text(if (create) "Create your account" else "Welcome back", style = MaterialTheme.typography.headlineSmall, color = T.text)
            Muted("All your devices, working together. Privately.")
            Spacer(Modifier.height(4.dp))

            Field(user, { user = it }, "Username")
            Field(pass, { pass = it }, "Password", password = true)
            Row(verticalAlignment = Alignment.CenterVertically) {
                Field(key, { key = it }, "Device key", Modifier.weight(1f), password = true)
                if (create) {
                    GhostButton("Generate", { key = generateKey() }, Modifier.padding(start = 8.dp), small = true)
                }
            }
            Muted("Use the same key on all your devices. It encrypts everything they send each other.")

            TextButton(onClick = { advanced = !advanced }) {
                Text(if (advanced) "Hide advanced" else "Advanced", color = T.text2)
            }
            if (advanced) Field(server, { server = it }, "Server")

            if (error.isNotEmpty()) ErrorText(error)

            PrimaryButton(
                if (busy) "Signing in…" else if (create) "Create account" else "Sign in",
                ::submit, Modifier.fillMaxWidth(), enabled = !busy,
            )
            Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.Center, verticalAlignment = Alignment.CenterVertically) {
                Muted(if (create) "Already have an account?" else "New here?")
                TextButton(onClick = { create = !create; error = "" }) {
                    Text(if (create) "Sign in" else "Create an account", color = T.text)
                }
            }
        }
    }
}
