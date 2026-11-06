package com.mydomain.android.ui

import androidx.compose.foundation.layout.heightIn
import androidx.compose.ui.semantics.Role
import androidx.compose.foundation.selection.selectableGroup
import android.content.Context
import android.content.Intent
import android.content.SharedPreferences
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
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
import androidx.compose.runtime.LaunchedEffect
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
import com.mydomain.android.CameraShare
import com.mydomain.android.NotifListener
import com.mydomain.android.PhoneHelper
import com.mydomain.android.PhoneLocation
import com.mydomain.android.Privileged
import com.mydomain.android.R
import com.mydomain.android.SetupState
import com.mydomain.android.WifiDirect
import com.mydomain.android.core.ChatHistory
import com.mydomain.android.core.ClipLog
import com.mydomain.android.core.ClipPrivacy
import com.mydomain.android.core.Core
import com.mydomain.android.core.friendlyError
import com.mydomain.android.core.niceName
import kotlinx.coroutines.delay
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

/** "Keep message history" ([ChatHistory]): on by default; turning it off
 *  forgets every message kept and stops saving. */
@Composable
fun ChatHistorySwitch() {
    val ctx = LocalContext.current
    SwitchRow(stringResource(R.string.settings_chat_history), ChatHistory.enabled) { ChatHistory.setEnabled(ctx, it) }
    Muted(stringResource(R.string.settings_chat_history_hint))
}

/** "Keep a clipboard history" ([ClipLog]): on by default; turning it off
 *  forgets every clip kept. */
@Composable
fun ClipHistorySwitch() {
    val ctx = LocalContext.current
    SwitchRow(stringResource(R.string.settings_clip_history), ClipLog.enabled) { ClipLog.setEnabled(ctx, it) }
    Muted(stringResource(R.string.settings_clip_history_hint))
}

/** "Never sync passwords and other concealed clips" ([ClipPrivacy]): on
 *  by default. */
@Composable
fun HideConcealedSwitch() {
    val ctx = LocalContext.current
    var on by remember { mutableStateOf(ClipPrivacy.hideConcealed(ctx)) }
    SwitchRow(stringResource(R.string.settings_clip_hide_concealed), on) {
        on = it
        ClipPrivacy.setHideConcealed(ctx, it)
    }
    Muted(stringResource(R.string.settings_clip_hide_concealed_hint))
}

/** "Share names and icons with my other devices": the nicknames, icons
 *  and colours given to devices ([com.mydomain.android.core.DeviceLabels])
 *  go to your other devices and theirs come here. On by default. */
@Composable
fun ShareLabelsSwitch() {
    var on by remember { mutableStateOf(Core.labelsShare()) }
    SwitchRow(stringResource(R.string.settings_share_labels), on) {
        on = it
        Core.setLabelsShare(it)
    }
    Muted(stringResource(R.string.settings_share_labels_hint))
}

/** Photo access, the photo backup (on/off, which computer, charging only)
 *  and battery alerts: the phone has no other place to turn these on. */
@Composable
fun PhotosTile() {
    val ctx = LocalContext.current
    val toast = LocalToast.current
    val scope = rememberCoroutineScope()
    var allowed by remember { mutableStateOf(com.mydomain.android.Gallery.canRead(ctx)) }
    val ask = androidx.activity.compose.rememberLauncherForActivityResult(
        androidx.activity.result.contract.ActivityResultContracts.RequestMultiplePermissions(),
    ) {
        allowed = com.mydomain.android.Gallery.canRead(ctx)
        com.mydomain.android.FileActions.shareFolders(ctx)
        com.mydomain.android.PhotoBackup.scanSoon()
    }
    androidx.compose.runtime.LaunchedEffect(Unit) { Core.refreshBackup() }
    val b = Core.backup
    var picking by remember { mutableStateOf(false) }
    Tile(title = stringResource(R.string.settings_photos)) {
        Muted(stringResource(R.string.settings_photos_hint))
        if (!allowed) GhostButton(stringResource(R.string.settings_allow_photos), { ask.launch(com.mydomain.android.Gallery.permissions()) })
        val set = { enabled: Boolean, target: String, charging: Boolean ->
            scope.launch {
                try { Core.setBackup(enabled, target, charging) } catch (e: Exception) { toast(friendlyError(e)) }
            }
        }
        SwitchRow(stringResource(R.string.settings_backup), b?.enabled == true) { on ->
            val target = b?.target.orEmpty()
            if (on && target.isEmpty()) picking = true else set(on, target, b?.chargingOnly == true)
        }
        if (b?.enabled == true || picking) {
            val current = Core.peers.firstOrNull { it.nodeId == b?.target }
            if (current != null && !picking) {
                GhostButton(String.format(stringResource(R.string.settings_backup_to), Core.peerName(current.nodeId)), { picking = true }, small = true)
            } else {
                Muted(stringResource(R.string.settings_backup_pick))
                Core.peers.forEach { p ->
                    SoftButton(Core.peerName(p.nodeId), {
                        picking = false
                        set(true, p.nodeId, b?.chargingOnly == true)
                    }, Modifier.fillMaxWidth())
                }
            }
            SwitchRow(stringResource(R.string.settings_backup_charging), b?.chargingOnly == true) { on ->
                set(b?.enabled == true, b?.target.orEmpty(), on)
            }
            b?.takeIf { it.enabled }?.let { Muted(backupLine(it), Modifier.announce()) }
            Muted(stringResource(R.string.settings_backup_hint))
        }
        var alerts by remember { mutableStateOf(Core.batteryAlerts()?.enabled ?: true) }
        var charged by remember { mutableIntStateOf(Core.batteryAlerts()?.charged?.toInt() ?: 0) }
        val low = { Core.batteryAlerts()?.low?.toInt() ?: 15 }
        SwitchRow(stringResource(R.string.settings_battery_alerts), alerts) { on ->
            alerts = on
            scope.launch { runCatching { Core.setBatteryAlerts(on, low(), charged) } }
        }
        if (alerts) {
            Muted(stringResource(R.string.settings_battery_charged))
            Row(Modifier.fillMaxWidth().selectableGroup(), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                listOf(0, 80, 90, 100).forEach { pct ->
                    val label = if (pct == 0) stringResource(R.string.settings_battery_charged_off)
                    else stringResource(R.string.settings_battery_charged_pct, pct)
                    val pick: () -> Unit = {
                        charged = pct
                        scope.launch {
                            try { Core.setBatteryAlerts(true, low(), pct) } catch (e: Exception) { toast(friendlyError(e)) }
                        }
                    }
                    // The chosen one is filled, and says "selected".
                    if (pct == charged) PrimaryButton(label, pick, Modifier.weight(1f), selected = true)
                    else SoftButton(label, pick, Modifier.weight(1f), selected = false)
                }
            }
            Muted(stringResource(R.string.settings_battery_charged_hint))
        }
    }
}

/** Touchpad: block screenshots while the full-screen pad is open, so a
 *  phone's three-finger screenshot swipe doesn't fire over it. */
@Composable
fun TouchpadSettingsTile() {
    val prefs = LocalContext.current.getSharedPreferences("md", Context.MODE_PRIVATE)
    var on by remember { mutableStateOf(prefs.getBoolean(PREF_TOUCHPAD_SECURE, true)) }
    Tile(title = stringResource(R.string.touchpad_title)) {
        SwitchRow(stringResource(R.string.settings_touchpad_secure), on) {
            on = it
            prefs.edit().putBoolean(PREF_TOUCHPAD_SECURE, it).apply()
        }
        Muted(stringResource(R.string.settings_touchpad_secure_hint))
    }
}

/** "Let my computers see where this phone is" (PhoneLocation): off until
 *  you turn it on here. */
@Composable
fun LocationSettingsTile() {
    val ctx = LocalContext.current
    var on by remember { mutableStateOf(PhoneLocation.allowed(ctx)) }
    Tile(title = stringResource(R.string.location_title)) {
        SwitchRow(stringResource(R.string.location_switch), on) {
            on = it
            PhoneLocation.setAllowed(ctx, it)
        }
        Muted(stringResource(R.string.location_hint))
    }
}

/** "Use Wi-Fi Direct for big files when nearby" (WifiDirect), on by
 *  default: phone to phone, to a Windows PC, or this phone's own hotspot
 *  for a Mac or Linux computer. */
@Composable
fun WifiDirectTile() {
    val ctx = LocalContext.current
    var on by remember { mutableStateOf(WifiDirect.enabled(ctx)) }
    Tile(title = stringResource(R.string.wifi_direct_title)) {
        SwitchRow(stringResource(R.string.wifi_direct_switch), on) {
            on = it
            WifiDirect.setEnabled(ctx, it)
        }
        Muted(stringResource(R.string.wifi_direct_hint))
    }
}

/** The camera on your computers (CameraShare): "Keep streaming while
 *  locked", off by default (the camera stops as the phone locks). */
@Composable
fun CameraSettingsTile() {
    val ctx = LocalContext.current
    var on by remember { mutableStateOf(CameraShare.keepWhileLocked(ctx)) }
    Tile(title = stringResource(R.string.camera_settings_title)) {
        SwitchRow(stringResource(R.string.camera_keep_locked), on) {
            on = it
            CameraShare.setKeepWhileLocked(ctx, it)
        }
        Muted(stringResource(R.string.camera_keep_locked_hint))
    }
}

/** "Use nearby mode for media" (NearbyMode), on by default: a stream
 *  with a device that's near moves to a direct local link. */
@Composable
fun NearbySettingsTile() {
    val ctx = LocalContext.current
    var on by remember { mutableStateOf(com.mydomain.android.NearbyMode.enabled(ctx)) }
    Tile(title = stringResource(R.string.nearby_settings_title)) {
        SwitchRow(stringResource(R.string.nearby_switch), on) {
            on = it
            com.mydomain.android.NearbyMode.set(ctx, it)
        }
        Muted(stringResource(R.string.nearby_switch_hint))
    }
}

/** Print from the phone on a computer's printer (PrintToComputer): which
 *  computers print for it, and the way to Android's printing settings,
 *  where the print service is turned on. */
@Composable
fun PrintSettingsTile() {
    val ctx = LocalContext.current
    var computers by remember { mutableStateOf(emptyList<String>()) }
    // A computer turning printing on or off says so over its link.
    LaunchedEffect(Unit) {
        while (true) {
            val engine = Core.engine()
            computers = Core.peers
                .filter { p -> runCatching { engine?.printSupported(p.nodeId) == true }.getOrDefault(false) }
                .map { Core.peerName(it.nodeId) }
            delay(3_000)
        }
    }
    Tile(title = stringResource(R.string.print_title)) {
        Muted(stringResource(R.string.print_hint))
        Text(
            if (computers.isEmpty()) stringResource(R.string.print_no_computers)
            else stringResource(R.string.print_computers, computers.joinToString(", ")),
            Modifier.announce(),
            style = MaterialTheme.typography.bodyMedium, color = T.text,
        )
        SoftButton(stringResource(R.string.print_open_settings), onClick = {
            runCatching { ctx.startActivity(Intent(android.provider.Settings.ACTION_PRINT_SETTINGS)) }
        })
    }
}

/** The Accessibility service (PhoneHelper): radios switched and keys
 *  pressed for your computers, on any phone. */
@Composable
fun HelperSettingsTile() {
    val ctx = LocalContext.current
    var on by remember { mutableStateOf(PhoneHelper.enabled(ctx)) }
    // Turned on in Android's Settings: look again every few seconds.
    LaunchedEffect(Unit) {
        while (true) {
            on = PhoneHelper.enabled(ctx)
            delay(2_000)
        }
    }
    Tile(title = stringResource(R.string.helper_title)) {
        Muted(stringResource(R.string.helper_hint))
        Text(
            stringResource(if (on) R.string.helper_on else R.string.helper_off), Modifier.announce(),
            style = MaterialTheme.typography.bodyMedium, color = T.text,
        )
        SoftButton(stringResource(R.string.helper_open), onClick = {
            runCatching { ctx.startActivity(Intent(android.provider.Settings.ACTION_ACCESSIBILITY_SETTINGS)) }
        })
    }
}

/** Shizuku (ADR-0006): lets your computers switch this phone's Wi-Fi,
 *  Bluetooth, airplane mode and mobile data with no tap here. Says what's
 *  missing, with the one button that fixes it. */
@Composable
fun ShizukuSettingsTile() {
    val ctx = LocalContext.current
    var state by remember { mutableStateOf(Privileged.state()) }
    // Shizuku starts and stops in its own app: look again every few seconds.
    LaunchedEffect(Unit) {
        while (true) {
            state = Privileged.state()
            delay(2_000)
        }
    }
    Tile(title = stringResource(R.string.shizuku_title)) {
        Muted(stringResource(R.string.shizuku_hint))
        Text(
            stringResource(
                when (state) {
                    Privileged.State.NOT_INSTALLED -> R.string.shizuku_not_installed
                    Privileged.State.NOT_RUNNING -> R.string.shizuku_not_running
                    Privileged.State.NOT_ALLOWED -> R.string.shizuku_not_allowed
                    Privileged.State.READY -> R.string.shizuku_ready
                },
            ),
            Modifier.announce(), style = MaterialTheme.typography.bodyMedium, color = T.text,
        )
        when (state) {
            Privileged.State.NOT_INSTALLED ->
                SoftButton(stringResource(R.string.shizuku_get), onClick = { SetupState.openShizuku(ctx) })
            Privileged.State.NOT_RUNNING ->
                SoftButton(stringResource(R.string.shizuku_open), onClick = { SetupState.openShizuku(ctx) })
            Privileged.State.NOT_ALLOWED ->
                SoftButton(stringResource(R.string.shizuku_allow), onClick = { SetupState.openShizuku(ctx) })
            Privileged.State.READY -> {}
        }
    }
}

/** One line on what the backup is doing. */
@Composable
private fun backupLine(b: com.mydomain.android.ffi.BackupInfo): String {
    val waiting = b.queued.toInt()
    return when (b.waitingFor) {
        "wifi" -> stringResource(R.string.backup_status_wifi, waiting)
        "charging" -> stringResource(R.string.backup_status_charging, waiting)
        "battery" -> stringResource(R.string.backup_status_battery, waiting)
        "retry" -> stringResource(R.string.backup_status_retry, b.lastError.orEmpty())
        else -> b.current?.let { name ->
            val pct = if (b.total > 0u) (b.done.toLong() * 100 / b.total.toLong()).toInt() else 0
            stringResource(R.string.backup_status_sending, name, pct, waiting)
        } ?: stringResource(R.string.backup_status_done, b.sent.toInt())
    }
}

/** Account, this device, and preferences. Tap the version 7 times for the
 *  Developer screen. */
@Composable
fun SettingsScreen(
    prefs: SharedPreferences,
    onDeveloper: () -> Unit,
    onAutomations: () -> Unit = {},
    onSetup: () -> Unit = {},
    onHelp: () -> Unit = {},
    focus: String? = null,
) {
    // [focus]: the tile a search result opened (SearchScreen's Settings ids),
    // marked and brought into view.
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

        SetupSettingsTile(onSetup)

        Tile(Modifier.foundMark(focus == "help"), title = stringResource(R.string.help_title), onClick = onHelp) {
            Muted(stringResource(R.string.help_settings_hint))
        }

        Tile(Modifier.foundMark(focus == "account"), title = stringResource(R.string.settings_account)) {
            Text(
                boldArg(R.string.settings_signed_in_as, Core.session?.username ?: prefs.getString("user", null) ?: "…"),
                style = MaterialTheme.typography.bodyMedium, color = T.text,
            )
            AddDevice()
            GhostButton(stringResource(R.string.settings_sign_out), { scope.launch { runCatching { Core.signOut() } } })
        }

        Tile(Modifier.foundMark(focus == "device"), title = stringResource(R.string.settings_this_device)) {
            Text(
                boldArg(R.string.settings_seen_as, niceName(Core.me?.name ?: "")),
                style = MaterialTheme.typography.bodyMedium, color = T.text,
            )
            OwnFingerprint()
            ClipSyncSwitch(stringResource(R.string.settings_clip_sync))
            ChatHistorySwitch()
            ClipHistorySwitch()
            HideConcealedSwitch()
            ShareCodesSwitch()
            ShareLabelsSwitch()
        }

        Tile(Modifier.foundMark(focus == "automations"), title = stringResource(R.string.automations_title)) {
            Muted(stringResource(R.string.automations_settings_hint))
            SoftButton(stringResource(R.string.automations_open), onAutomations)
        }

        Box(Modifier.foundMark(focus == "automation-apps")) { AutomationAppsTile() }

        Box(Modifier.foundMark(focus == "photos")) { PhotosTile() }
        Box(Modifier.foundMark(focus == "sync")) { SyncFoldersTile() }
        Box(Modifier.foundMark(focus == "battery")) { SaveBatteryTile() }
        Box(Modifier.foundMark(focus == "touchpad")) { TouchpadSettingsTile() }
        Box(Modifier.foundMark(focus == "location")) { LocationSettingsTile() }
        Box(Modifier.foundMark(focus == "camera")) { CameraSettingsTile() }
        Box(Modifier.foundMark(focus == "nearby")) { NearbySettingsTile() }
        Box(Modifier.foundMark(focus == "wifi-direct")) { WifiDirectTile() }
        Box(Modifier.foundMark(focus == "print")) { PrintSettingsTile() }
        Box(Modifier.foundMark(focus == "helper")) { HelperSettingsTile() }
        Box(Modifier.foundMark(focus == "shizuku")) { ShizukuSettingsTile() }

        Tile(Modifier.foundMark(focus == "key"), title = stringResource(R.string.settings_device_key)) {
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

        SettingsBackupTile(Modifier.foundMark(focus == "backup"))

        Tile(Modifier.foundMark(focus == "appearance"), title = stringResource(R.string.settings_appearance)) {
            Muted(stringResource(R.string.settings_appearance_hint))
            ThemePicker(prefs)
            LanguageSetting()
        }

        Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.Center, verticalAlignment = Alignment.CenterVertically) {
            Muted(
                stringResource(R.string.settings_version, BuildConfig.VERSION_NAME),
                Modifier.heightIn(min = 48.dp).clickable(role = Role.Button) {
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

/** The app's language: it follows the phone's language (English or Hindi,
 *  res/values-hi). On Android 13+ the system's per-app language page
 *  (res/xml/locales_config.xml) can override it; this opens that page. */
@Composable
private fun LanguageSetting() {
    val ctx = LocalContext.current
    if (android.os.Build.VERSION.SDK_INT >= 33) {
        Muted(stringResource(R.string.settings_language_hint))
        GhostButton(stringResource(R.string.settings_language), {
            val pkg = android.net.Uri.fromParts("package", ctx.packageName, null)
            try {
                ctx.startActivity(Intent(android.provider.Settings.ACTION_APP_LOCALE_SETTINGS, pkg))
            } catch (_: Exception) {
                runCatching { ctx.startActivity(Intent(android.provider.Settings.ACTION_APPLICATION_DETAILS_SETTINGS, pkg)) }
            }
        })
    } else {
        Muted(stringResource(R.string.settings_language_hint_old))
    }
}
