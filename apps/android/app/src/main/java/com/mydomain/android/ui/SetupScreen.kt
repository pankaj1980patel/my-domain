package com.mydomain.android.ui

import android.app.Activity
import android.content.Context
import android.content.pm.PackageManager
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.annotation.StringRes
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.core.app.ActivityCompat
import androidx.core.content.ContextCompat
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleEventObserver
import com.mydomain.android.PhoneLocation
import com.mydomain.android.R
import com.mydomain.android.SetupState
import com.mydomain.android.core.SetupChecklist
import com.mydomain.android.core.SetupItem
import com.mydomain.android.core.SetupRow
import com.mydomain.android.core.SetupStatus
import kotlinx.coroutines.delay

/** The checklist's rows, read again whenever the app comes back to the
 *  foreground (e.g. from Android's Settings) and on [refresh]. */
class SetupRows(private val ctx: Context) {
    var rows by mutableStateOf(SetupState.rows(ctx))
        private set

    fun refresh() {
        rows = SetupState.rows(ctx)
    }

    /** "N of M on". */
    val count: Pair<Int, Int> get() = SetupChecklist.count(rows)
}

@Composable
fun rememberSetupRows(): SetupRows {
    val ctx = LocalContext.current
    val state = remember { SetupRows(ctx) }
    // androidx.lifecycle.compose's LocalLifecycleOwner isn't a direct
    // dependency; Compose UI's (deprecated) alias is the same object.
    @Suppress("DEPRECATION")
    val owner = androidx.compose.ui.platform.LocalLifecycleOwner.current
    DisposableEffect(owner) {
        val watch = LifecycleEventObserver { _, event ->
            if (event == Lifecycle.Event.ON_RESUME) state.refresh()
        }
        owner.lifecycle.addObserver(watch)
        onDispose { owner.lifecycle.removeObserver(watch) }
    }
    return state
}

@StringRes
private fun titleOf(item: SetupItem): Int = when (item) {
    SetupItem.NOTIFICATIONS -> R.string.setup_notifications_title
    SetupItem.NOTIFICATION_ACCESS -> R.string.setup_notif_access_title
    SetupItem.ACCESSIBILITY_HELPER -> R.string.setup_helper_title
    SetupItem.BATTERY -> R.string.setup_battery_title
    SetupItem.NEARBY_DEVICES -> R.string.setup_nearby_title
    SetupItem.LOCATION_SHARING -> R.string.setup_location_sharing_title
    SetupItem.LOCATION_PERMISSION -> R.string.setup_location_title
    SetupItem.PHOTOS -> R.string.setup_photos_title
    SetupItem.WRITE_SETTINGS -> R.string.setup_write_settings_title
    SetupItem.FULL_SCREEN -> R.string.setup_full_screen_title
    SetupItem.SHIZUKU -> R.string.setup_shizuku_title
    SetupItem.WIDGET -> R.string.setup_widget_title
    SetupItem.QUICK_TILES -> R.string.setup_tiles_title
}

@StringRes
private fun purposeOf(item: SetupItem): Int = when (item) {
    SetupItem.NOTIFICATIONS -> R.string.setup_notifications_for
    SetupItem.NOTIFICATION_ACCESS -> R.string.setup_notif_access_for
    SetupItem.ACCESSIBILITY_HELPER -> R.string.setup_helper_for
    SetupItem.BATTERY -> R.string.setup_battery_for
    SetupItem.NEARBY_DEVICES -> R.string.setup_nearby_for
    SetupItem.LOCATION_SHARING -> R.string.setup_location_sharing_for
    SetupItem.LOCATION_PERMISSION -> R.string.setup_location_for
    SetupItem.PHOTOS -> R.string.setup_photos_for
    SetupItem.WRITE_SETTINGS -> R.string.setup_write_settings_for
    SetupItem.FULL_SCREEN -> R.string.setup_full_screen_for
    SetupItem.SHIZUKU -> R.string.setup_shizuku_for
    SetupItem.WIDGET -> R.string.setup_widget_for
    SetupItem.QUICK_TILES -> R.string.setup_tiles_for
}

/** "Set up this phone": every optional permission and setting, what it's
 *  for, whether it's on, and the button that opens Android's screen for
 *  it. The app never turns any of them on itself. */
@Composable
fun SetupScreen(onBack: () -> Unit) {
    val ctx = LocalContext.current
    val toast = LocalToast.current
    val setup = rememberSetupRows()
    val noScreen = stringResource(R.string.setup_no_screen)
    // Accessibility and Shizuku change outside any prompt: look again every
    // few seconds while this is on screen (cheap reads).
    LaunchedEffect(Unit) {
        while (true) {
            delay(3_000)
            setup.refresh()
        }
    }
    var asking by remember { mutableStateOf<SetupItem?>(null) }
    val ask = rememberLauncherForActivityResult(ActivityResultContracts.RequestMultiplePermissions()) { result ->
        val item = asking
        asking = null
        if (item != null) {
            SetupState.onAnswered(ctx, item)
            // Android shows no prompt after "Don't allow" twice: send the
            // user to this app's permissions page instead.
            val denied = result.isNotEmpty() && result.values.none { it }
            val activity = ctx as? Activity
            val silenced = activity != null && result.keys.none { ActivityCompat.shouldShowRequestPermissionRationale(activity, it) }
            if (denied && silenced) SetupState.openAppDetails(ctx)
        }
        setup.refresh()
    }
    val turnOn = { item: SetupItem ->
        val perms = SetupState.permissions(item)
        val askable = perms != null &&
            perms.any { ContextCompat.checkSelfPermission(ctx, it) != PackageManager.PERMISSION_GRANTED }
        if (askable) {
            asking = item
            ask.launch(perms)
        } else if (!SetupState.open(ctx, item)) {
            toast(noScreen)
        }
        setup.refresh()
    }

    Column(
        Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(start = 16.dp, end = 16.dp, bottom = 16.dp),
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        Row(Modifier.padding(top = 4.dp), verticalAlignment = Alignment.CenterVertically) {
            IconButton(onClick = onBack) {
                Icon(Icons.AutoMirrored.Filled.ArrowBack, stringResource(R.string.back), tint = T.text)
            }
            Column {
                Text(stringResource(R.string.setup_title), style = MaterialTheme.typography.headlineSmall, color = T.text)
                val (on, total) = setup.count
                Muted(stringResource(R.string.setup_count, on, total))
            }
        }
        Muted(stringResource(R.string.setup_hint))
        setup.rows.forEach { row ->
            SetupItemTile(row, onTurnOn = { turnOn(row.item) }, onSwitch = { on ->
                PhoneLocation.setAllowed(ctx, on)
                setup.refresh()
            })
        }
    }
}

@Composable
private fun SetupItemTile(row: SetupRow, onTurnOn: () -> Unit, onSwitch: (Boolean) -> Unit) {
    val ctx = LocalContext.current
    Tile(title = stringResource(titleOf(row.item))) {
        Muted(stringResource(purposeOf(row.item)))
        val (label, color) = when (row.status) {
            SetupStatus.ON -> stringResource(R.string.setup_on) to T.success
            SetupStatus.OFF -> stringResource(R.string.setup_off) to T.warning
            SetupStatus.NOT_NEEDED -> stringResource(R.string.setup_not_needed) to T.text3
            SetupStatus.INFO -> stringResource(R.string.setup_info) to T.text3
        }
        when {
            // An in-app choice, not an Android permission: the switch itself.
            row.item == SetupItem.LOCATION_SHARING ->
                SwitchRow(stringResource(R.string.location_switch), row.status == SetupStatus.ON, onSwitch)
            else -> Text(label, style = MaterialTheme.typography.labelLarge, fontWeight = FontWeight.Bold, color = color)
        }
        if (row.status == SetupStatus.OFF) {
            when (row.item) {
                SetupItem.LOCATION_SHARING -> {}
                SetupItem.WIDGET ->
                    if (SetupState.canPinWidget(ctx)) SoftButton(stringResource(R.string.setup_widget_add), onTurnOn)
                    else Muted(stringResource(R.string.setup_widget_how))
                else -> SoftButton(stringResource(R.string.setup_turn_on), onTurnOn)
            }
        }
    }
}

/** Settings' top tile: "Set up this phone · N of M on", opens the checklist. */
@Composable
fun SetupSettingsTile(onOpen: () -> Unit) {
    val setup = rememberSetupRows()
    val (on, total) = setup.count
    Tile(title = stringResource(R.string.setup_tile, on, total), onClick = onOpen) {
        Muted(stringResource(R.string.setup_tile_hint))
    }
}

/** The home screen's card, shown once after sign-in until it's dismissed
 *  or opened (remembered in the "md" prefs), and only while something is
 *  still off. */
@Composable
fun SetupHomeCard(onOpen: () -> Unit) {
    val ctx = LocalContext.current
    val prefs = remember { ctx.getSharedPreferences("md", Context.MODE_PRIVATE) }
    var dismissed by remember { mutableStateOf(prefs.getBoolean(SetupState.PREF_CARD_DISMISSED, false)) }
    if (dismissed) return
    val setup = rememberSetupRows()
    val (on, total) = setup.count
    if (!SetupChecklist.showCard(signedIn = true, dismissed = dismissed, on = on, total = total)) return
    val dismiss = {
        dismissed = true
        prefs.edit().putBoolean(SetupState.PREF_CARD_DISMISSED, true).apply()
    }
    Tile(title = stringResource(R.string.setup_card_title)) {
        Muted(stringResource(R.string.setup_card_body, on, total))
        Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            PrimaryButton(stringResource(R.string.setup_card_open), {
                dismiss()
                onOpen()
            }, Modifier.weight(1f))
            GhostButton(stringResource(R.string.setup_card_dismiss), dismiss, Modifier.weight(1f))
        }
    }
}
