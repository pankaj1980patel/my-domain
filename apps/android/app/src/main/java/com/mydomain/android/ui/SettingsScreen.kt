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
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import com.mydomain.android.BuildConfig
import com.mydomain.android.NotifListener
import com.mydomain.android.R
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

/** Sending verification codes found in notifications to the computers
 *  (NotifListener reads the same "share_codes" pref). Off by default. */
@Composable
fun ShareCodesSwitch() {
    val prefs = LocalContext.current.getSharedPreferences("md", Context.MODE_PRIVATE)
    var on by remember { mutableStateOf(prefs.getBoolean(NotifListener.PREF_SHARE_CODES, false)) }
    SwitchRow(stringResource(R.string.settings_share_codes), on) {
        on = it
        prefs.edit().putBoolean(NotifListener.PREF_SHARE_CODES, it).apply()
    }
    Muted(stringResource(R.string.settings_share_codes_hint))
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
    val keyChanged = stringResource(R.string.settings_key_changed)
    val unlocked = stringResource(R.string.settings_developer_unlocked)

    Column(
        Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(16.dp),
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        PageHead(stringResource(R.string.settings_title))

        Tile(title = stringResource(R.string.settings_account)) {
            Text(
                boldArg(R.string.settings_signed_in_as, Core.session?.username ?: prefs.getString("user", null) ?: "…"),
                style = MaterialTheme.typography.bodyMedium, color = T.text,
            )
            GhostButton(stringResource(R.string.settings_sign_out), { scope.launch { runCatching { Core.signOut() } } })
        }

        Tile(title = stringResource(R.string.settings_this_device)) {
            Text(
                boldArg(R.string.settings_seen_as, niceName(Core.me?.name ?: "")),
                style = MaterialTheme.typography.bodyMedium, color = T.text,
            )
            ClipSyncSwitch(stringResource(R.string.settings_clip_sync))
            ShareCodesSwitch()
        }

        Tile(title = stringResource(R.string.settings_device_key)) {
            Muted(stringResource(R.string.settings_key_hint))
            if (!changing) {
                GhostButton(stringResource(R.string.settings_change_key), { changing = true })
            } else {
                Field(newKey, { newKey = it }, stringResource(R.string.settings_new_key), password = true)
                Field(pass, { pass = it }, stringResource(R.string.settings_your_password), password = true)
                Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    PrimaryButton(stringResource(R.string.save), {
                        scope.launch {
                            try {
                                Core.call { it.updateEncryptionKey(newKey, pass) }
                                toast(keyChanged)
                                changing = false; newKey = ""; pass = ""
                            } catch (e: Exception) {
                                toast(friendlyError(e))
                            }
                        }
                    }, enabled = newKey.isNotEmpty() && pass.isNotEmpty())
                    GhostButton(stringResource(R.string.cancel), { changing = false })
                }
            }
        }

        Tile(title = stringResource(R.string.settings_appearance)) {
            Muted(stringResource(R.string.settings_appearance_hint))
            ThemePicker(prefs)
        }

        Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.Center, verticalAlignment = Alignment.CenterVertically) {
            Muted(
                stringResource(R.string.settings_version, BuildConfig.VERSION_NAME),
                Modifier.clickable {
                    taps += 1
                    if (taps >= 7) {
                        taps = 0
                        toast(unlocked)
                        onDeveloper()
                    }
                }.padding(12.dp),
            )
        }
    }
}
