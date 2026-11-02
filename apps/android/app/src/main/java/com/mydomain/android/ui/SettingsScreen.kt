package com.mydomain.android.ui

import android.content.Context
import android.content.SharedPreferences
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.withStyle
import androidx.compose.ui.unit.dp
import com.mydomain.android.BuildConfig
import com.mydomain.android.core.Core
import com.mydomain.android.core.friendlyError
import com.mydomain.android.core.niceName
import kotlinx.coroutines.launch

/** The clipboard auto-sync switch; the choice survives restarts. */
@Composable
fun ClipSyncSwitch(label: String) {
    val prefs = LocalContext.current.getSharedPreferences("md", Context.MODE_PRIVATE)
    SwitchRow(label, Core.clipSync) { on ->
        Core.enableClipSync(on)
        prefs.edit().putBoolean("clip_sync", on).apply()
    }
}

/** Account, this device, and preferences. Tap the version 7 times for the
 *  Developer screen. */
@Composable
fun SettingsScreen(prefs: SharedPreferences, onDeveloper: () -> Unit) {
    val toast = LocalToast.current
    val scope = rememberCoroutineScope()
    var taps by remember { mutableIntStateOf(0) }
    var changing by remember { mutableStateOf(false) }
    var newKey by remember { mutableStateOf("") }
    var pass by remember { mutableStateOf("") }

    Column(
        Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(16.dp),
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        PageHead("Settings")

        Tile(title = "Account") {
            Text(
                buildAnnotatedString {
                    append("Signed in as ")
                    withStyle(SpanStyle(fontWeight = FontWeight.SemiBold)) {
                        append(Core.session?.username ?: prefs.getString("user", null) ?: "…")
                    }
                },
                style = MaterialTheme.typography.bodyMedium, color = T.text,
            )
            GhostButton("Sign out", { scope.launch { runCatching { Core.signOut() } } })
        }

        Tile(title = "This device") {
            Text(
                buildAnnotatedString {
                    append("Other devices see it as ")
                    withStyle(SpanStyle(fontWeight = FontWeight.SemiBold)) { append(niceName(Core.me?.name ?: "")) }
                    append(".")
                },
                style = MaterialTheme.typography.bodyMedium, color = T.text,
            )
            ClipSyncSwitch("Keep clipboards in sync with my devices")
        }

        Tile(title = "Device key") {
            Muted("All your devices must use the same key. Changing it here means changing it on every device.")
            if (!changing) {
                GhostButton("Change device key", { changing = true })
            } else {
                Field(newKey, { newKey = it }, "New device key", password = true)
                Field(pass, { pass = it }, "Your password", password = true)
                Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    PrimaryButton("Save", {
                        scope.launch {
                            try {
                                Core.call { it.updateEncryptionKey(newKey, pass) }
                                toast("Device key changed")
                                changing = false; newKey = ""; pass = ""
                            } catch (e: Exception) {
                                toast(friendlyError(e))
                            }
                        }
                    }, enabled = newKey.isNotEmpty() && pass.isNotEmpty())
                    GhostButton("Cancel", { changing = false })
                }
            }
        }

        Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.Center, verticalAlignment = Alignment.CenterVertically) {
            Muted(
                "my-domain ${BuildConfig.VERSION_NAME}",
                Modifier.clickable {
                    taps += 1
                    if (taps >= 7) {
                        taps = 0
                        toast("Developer screen unlocked")
                        onDeveloper()
                    }
                }.padding(12.dp),
            )
        }
    }
}
