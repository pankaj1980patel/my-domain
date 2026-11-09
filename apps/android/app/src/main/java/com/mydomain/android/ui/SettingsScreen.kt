package com.mydomain.android.ui

import androidx.compose.foundation.layout.height
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.TextButton
import androidx.compose.ui.res.pluralStringResource
import androidx.compose.runtime.key
import com.mydomain.android.core.Health
import androidx.compose.material3.Badge
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.Spacer
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.activity.compose.BackHandler
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
 *  (NotifListener reads the same "share_codes" pref). Off by default; when
 *  on, which of your computers get them (all, until you choose). */
@Composable
fun ShareCodesSwitch() {
    val prefs = LocalContext.current.getSharedPreferences("md", Context.MODE_PRIVATE)
    var on by remember { mutableStateOf(prefs.getBoolean(NotifListener.PREF_SHARE_CODES, false)) }
    SwitchRow(stringResource(R.string.settings_share_codes), on) {
        on = it
        prefs.edit().putBoolean(NotifListener.PREF_SHARE_CODES, it).apply()
    }
    if (on) {
        val computers = remember {
            NotifListener.codeComputers(Core.engine()?.verificationCodeComputersJson() ?: "[]")
        }
        var chosen by remember { mutableStateOf(NotifListener.chosenComputers(prefs)) }
        if (computers.isEmpty()) {
            Muted(stringResource(R.string.settings_share_codes_none))
        } else {
            Muted(stringResource(R.string.settings_share_codes_pick))
            computers.forEach { (node, supported) ->
                val name = Core.peerName(node)
                val label = if (supported) name else stringResource(R.string.settings_share_codes_old, name)
                SwitchRow(label, supported && (chosen == null || node in chosen!!)) { pick ->
                    val now = (chosen ?: computers.filter { it.second }.map { it.first }.toSet()).toMutableSet()
                    if (pick) now += node else now -= node
                    chosen = now
                    prefs.edit().putStringSet(NotifListener.PREF_CODE_COMPUTERS, now).apply()
                }
            }
        }
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

/** "Share my wallpaper with my devices" (PhoneWallpaper), on by default:
 *  your computers draw this phone's tile from its wallpaper. */
@Composable
fun WallpaperSettingsTile() {
    val ctx = LocalContext.current
    var on by remember { mutableStateOf(com.mydomain.android.PhoneWallpaper.enabled(ctx)) }
    Tile(title = stringResource(R.string.wallpaper_title)) {
        SwitchRow(stringResource(R.string.wallpaper_switch), on) {
            on = it
            com.mydomain.android.PhoneWallpaper.set(ctx, it)
        }
        Muted(stringResource(R.string.wallpaper_hint))
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
            Core.awaitOnScreen()
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
        val open = rememberOpenScreen()
        SoftButton(stringResource(R.string.print_open_settings), onClick = {
            open(listOf(Intent(android.provider.Settings.ACTION_PRINT_SETTINGS)))
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
            Core.awaitOnScreen()
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
        val open = rememberOpenScreen()
        SoftButton(stringResource(R.string.helper_open), onClick = {
            open(listOf(com.mydomain.android.SecureMode.settingsIntent(ctx)))
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
            Core.awaitOnScreen()
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

/** A Settings group: its row on the first page, and the tiles (by the ids
 *  Search uses) its own page shows. */
private enum class SettingsGroup(
    val icon: Int,
    val hue: Hue,
    val title: Int,
    val sub: Int,
    val ids: Set<String>,
) {
    Account(
        R.drawable.ic_lucide_user, Hues.plain, R.string.set_group_account, R.string.set_group_account_sub,
        setOf("account", "sessions", "removed-devices", "key-rotation", "key"),
    ),
    Phone(
        R.drawable.ic_lucide_smartphone, Hues.plain, R.string.set_group_phone, R.string.set_group_phone_sub,
        setOf("device", "wallpaper", "location", "camera", "nearby", "wifi-direct", "touchpad"),
    ),
    Files(
        R.drawable.ic_lucide_folder, Hues.files, R.string.set_group_files, R.string.set_group_files_sub,
        setOf("photos", "sync", "print", "remote-download"),
    ),
    Automation(
        R.drawable.ic_lucide_zap, Hues.tools, R.string.automations_title, R.string.set_group_automation_sub,
        setOf("automations", "automation-apps"),
    ),
    Battery(
        R.drawable.ic_lucide_battery_charging, Hues.plain, R.string.set_group_battery, R.string.set_group_battery_sub,
        setOf("battery"),
    ),
    Helper(
        R.drawable.ic_lucide_shield, Hues.plain, R.string.set_group_helper, R.string.set_group_helper_sub,
        setOf("helper", "secure-mode", "shizuku"),
    ),
    // Setting up this phone. The health check itself is under More ->
    // Connections & health (its one place; the badge is on More).
    Health(
        R.drawable.ic_lucide_wrench, Hues.plain, R.string.set_group_setup, R.string.set_group_setup_sub,
        setOf("health", "setup"),
    ),
    Backup(
        R.drawable.ic_lucide_archive_restore, Hues.plain, R.string.set_group_backup, R.string.set_group_backup_sub,
        setOf("backup"),
    ),
    Appearance(
        R.drawable.ic_lucide_palette, Hues.plain, R.string.settings_appearance, R.string.set_group_appearance_sub,
        setOf("appearance"),
    ),
    ;

    companion object {
        /** The group a tile id is in, or `group:<name>` (debug screenshots). */
        fun of(id: String?): SettingsGroup? = id?.let { i ->
            if (i.startsWith("group:")) {
                entries.firstOrNull { it.name.equals(i.removePrefix("group:"), ignoreCase = true) }
            } else {
                entries.firstOrNull { i in it.ids }
            }
        }
    }
}

/** Settings: who is signed in, then grouped rows, each opening its own page
 *  of tiles (account, this phone, files, automations, battery, the phone
 *  helper, health, backup, appearance), then Help and the version (tap it 7
 *  times for the Developer screen). [focus]: the tile a search result
 *  opened (Search's Settings ids): its group's page opens with it marked. */
@Composable
fun SettingsScreen(
    prefs: SharedPreferences,
    onDeveloper: () -> Unit,
    onAutomations: () -> Unit = {},
    onSetup: () -> Unit = {},
    onHelp: () -> Unit = {},
    onHealth: () -> Unit = {},
    onOnboarding: () -> Unit = {},
    focus: String? = null,
    onBack: () -> Unit = {},
) {
    var taps by remember { mutableIntStateOf(0) }
    val unlocked = stringResource(R.string.settings_developer_unlocked)
    val toast = LocalToast.current
    var open by rememberSaveable(focus) { mutableStateOf(SettingsGroup.of(focus)?.name) }
    val group = open?.let { n -> SettingsGroup.entries.firstOrNull { it.name == n } }
    BackHandler(enabled = group != null) { open = null }

    Column(Modifier.fillMaxSize()) {
        BackBar(
            stringResource(group?.title ?: R.string.settings_title),
            onBack = { if (group != null) open = null else onBack() },
        )
        // Each page starts at its top (not where the list was scrolled to).
        key(open) {
        Column(
            Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(start = 16.dp, end = 16.dp, bottom = 16.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            if (group != null) {
                SettingsGroupPage(group, prefs, focus, onAutomations, onSetup, onHealth, onOnboarding)
            } else {
                // Who is signed in, at the top: opens Account & security.
                SoftCard(onClick = { open = SettingsGroup.Account.name }) {
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        IconBubble(R.drawable.ic_lucide_user, Hues.plain, 44.dp)
                        Spacer(Modifier.width(12.dp))
                        Column(Modifier.weight(1f)) {
                            Text(
                                Core.session?.username ?: prefs.getString("user", null) ?: "…",
                                style = MaterialTheme.typography.titleMedium, color = T.text,
                            )
                            Muted(stringResource(R.string.set_group_account))
                        }
                        Chevron()
                    }
                }
                RowGroup {
                    SettingsGroup.entries.filter { it != SettingsGroup.Account }.forEach { g ->
                        NavRow(g.icon, g.hue, stringResource(g.title), stringResource(g.sub), onClick = { open = g.name })
                    }
                }
                RowGroup {
                    NavRow(
                        R.drawable.ic_lucide_circle_help, Hue.Cyan, stringResource(R.string.help_title),
                        stringResource(R.string.help_settings_hint), onClick = onHelp, modifier = Modifier.foundMark(focus == "help"),
                    )
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
        }
    }
}

/** One group's page: its tiles, as they were on the one long page. */
@Composable
private fun SettingsGroupPage(
    group: SettingsGroup,
    prefs: SharedPreferences,
    focus: String?,
    onAutomations: () -> Unit,
    onSetup: () -> Unit,
    onHealth: () -> Unit,
    onOnboarding: () -> Unit,
) {
    val scope = rememberCoroutineScope()
    when (group) {
        SettingsGroup.Account -> {
            Tile(Modifier.foundMark(focus == "account"), title = stringResource(R.string.settings_account)) {
                Text(
                    boldArg(R.string.settings_signed_in_as, Core.session?.username ?: prefs.getString("user", null) ?: "…"),
                    style = MaterialTheme.typography.bodyMedium, color = T.text,
                )
                AddDevice()
                // The account password (not the encryption passphrase) and
                // signing out every device.
                AccountSecurity()
            }
            // Every sign-in to the account, each but this phone's with "Sign out".
            SessionsTile(Modifier.foundMark(focus == "sessions"))
            // Devices removed for good, each with "Allow again".
            RemovedDevicesTile(Modifier.foundMark(focus == "removed-devices"))
            // Change the passphrase on all devices (and this phone missing one).
            KeyRotationPrompt()
            KeyRotationTile(Modifier.foundMark(focus == "key-rotation"))
            DeviceKeyTile(Modifier.foundMark(focus == "key"))
            // Signing out: apart, last, in red, and asked first.
            Spacer(Modifier.height(12.dp))
            SignOutTile(scope)
        }
        SettingsGroup.Phone -> {
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
            Box(Modifier.foundMark(focus == "wallpaper")) { WallpaperSettingsTile() }
            Box(Modifier.foundMark(focus == "touchpad")) { TouchpadSettingsTile() }
            Box(Modifier.foundMark(focus == "location")) { LocationSettingsTile() }
            Box(Modifier.foundMark(focus == "camera")) { CameraSettingsTile() }
            Box(Modifier.foundMark(focus == "nearby")) { NearbySettingsTile() }
            Box(Modifier.foundMark(focus == "wifi-direct")) { WifiDirectTile() }
        }
        SettingsGroup.Files -> {
            Box(Modifier.foundMark(focus == "photos")) { PhotosTile() }
            Box(Modifier.foundMark(focus == "sync")) { SyncFoldersTile() }
            Box(Modifier.foundMark(focus == "print")) { PrintSettingsTile() }
            Box(Modifier.foundMark(focus == "remote-download")) { RemoteDownloadSettingsTile() }
        }
        SettingsGroup.Automation -> {
            Tile(Modifier.foundMark(focus == "automations"), title = stringResource(R.string.automations_title)) {
                Muted(stringResource(R.string.automations_settings_hint))
                SoftButton(stringResource(R.string.automations_open), onAutomations)
            }
            Box(Modifier.foundMark(focus == "automation-apps")) { AutomationAppsTile() }
        }
        SettingsGroup.Battery -> Box(Modifier.foundMark(focus == "battery")) { SaveBatteryTile() }
        SettingsGroup.Helper -> {
            Box(Modifier.foundMark(focus == "helper")) { HelperSettingsTile() }
            Box(Modifier.foundMark(focus == "secure-mode")) { SecureModeTile() }
            Box(Modifier.foundMark(focus == "shizuku")) { ShizukuSettingsTile() }
        }
        SettingsGroup.Health -> {
            // The checklist, and first-run setup again.
            SetupSettingsTile(onSetup)
            SetupAgainTile(onOnboarding)
        }
        SettingsGroup.Backup -> SettingsBackupTile(Modifier.foundMark(focus == "backup"))
        SettingsGroup.Appearance -> Tile(Modifier.foundMark(focus == "appearance"), title = stringResource(R.string.settings_appearance)) {
            Muted(stringResource(R.string.settings_appearance_hint))
            ThemePicker(prefs)
            LanguageSetting()
        }
    }
}

/** Sign this phone out (asks first; your other devices stay signed in). */
@Composable
private fun SignOutTile(scope: kotlinx.coroutines.CoroutineScope) {
    var asking by remember { mutableStateOf(false) }
    val toast = LocalToast.current
    val failed = stringResource(R.string.set_sign_out_failed)
    Tile(title = stringResource(R.string.settings_sign_out)) {
        Muted(stringResource(R.string.set_sign_out_hint))
        TextButton({ asking = true }, Modifier.heightIn(min = 48.dp)) {
            Text(stringResource(R.string.settings_sign_out), color = T.danger, style = MaterialTheme.typography.labelLarge)
        }
    }
    if (asking) {
        AlertDialog(
            onDismissRequest = { asking = false },
            containerColor = T.bar,
            title = { Text(stringResource(R.string.set_sign_out_title), color = T.text) },
            text = {
                Text(
                    stringResource(R.string.set_sign_out_hint) + "\n\n" + stringResource(R.string.set_sign_out_clears),
                    color = T.text2,
                )
            },
            dismissButton = { TextButton({ asking = false }) { Text(stringResource(R.string.cancel), color = T.text2) } },
            confirmButton = {
                TextButton({
                    asking = false
                    scope.launch { runCatching { Core.signOut() }.onFailure { toast(failed) } }
                }) { Text(stringResource(R.string.settings_sign_out), color = T.danger) }
            },
        )
    }
}

/** The encryption key: change it here (it must match on every device). */
@Composable
private fun DeviceKeyTile(modifier: Modifier) {
    val toast = LocalToast.current
    val scope = rememberCoroutineScope()
    var changing by remember { mutableStateOf(false) }
    var newKey by remember { mutableStateOf("") }
    var pass by remember { mutableStateOf("") }
    val keyChanged = stringResource(R.string.settings_key_changed)
    Tile(modifier, title = stringResource(R.string.settings_device_key)) {
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
}

/** The app's language: it follows the phone's language (English, Hindi,
 *  Marathi, Gujarati, Tamil or Spanish: res/values-XX). On Android 13+ the
 *  system's per-app language page (res/xml/locales_config.xml) can override
 *  it; this opens that page. */
@Composable
private fun LanguageSetting() {
    val ctx = LocalContext.current
    if (android.os.Build.VERSION.SDK_INT >= 33) {
        Muted(stringResource(R.string.settings_language_hint))
        val open = rememberOpenScreen()
        GhostButton(stringResource(R.string.settings_language), {
            val pkg = android.net.Uri.fromParts("package", ctx.packageName, null)
            open(
                listOf(
                    Intent(android.provider.Settings.ACTION_APP_LOCALE_SETTINGS, pkg),
                    Intent(android.provider.Settings.ACTION_APPLICATION_DETAILS_SETTINGS, pkg),
                ),
            )
        })
    } else {
        Muted(stringResource(R.string.settings_language_hint_old))
    }
}
