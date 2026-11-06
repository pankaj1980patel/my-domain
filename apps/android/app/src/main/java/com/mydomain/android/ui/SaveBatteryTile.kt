package com.mydomain.android.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.selection.selectableGroup
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import com.mydomain.android.PowerSaver
import com.mydomain.android.R
import com.mydomain.android.core.PowerPolicy

/** Save battery: the mode (Automatic / Always / Off), what "low battery"
 *  means, mobile data for big transfers, and what waits right now and why. */
@Composable
fun SaveBatteryTile() {
    val ctx = LocalContext.current
    LaunchedEffect(Unit) { PowerSaver.refresh(ctx) }
    val s = PowerSaver.settings
    Tile(title = stringResource(R.string.save_battery_title)) {
        Muted(stringResource(R.string.save_battery_hint))
        Choices(
            listOf(
                PowerPolicy.Mode.AUTO to stringResource(R.string.save_battery_mode_auto),
                PowerPolicy.Mode.ALWAYS to stringResource(R.string.save_battery_mode_always),
                PowerPolicy.Mode.OFF to stringResource(R.string.save_battery_mode_off),
            ),
            s.mode,
        ) { PowerSaver.setMode(ctx, it) }
        if (s.mode == PowerPolicy.Mode.AUTO) {
            Muted(stringResource(R.string.save_battery_threshold))
            val pcts = (PowerPolicy.THRESHOLDS + s.threshold).distinct().sorted()
            Choices(pcts.map { it to stringResource(R.string.settings_battery_charged_pct, it) }, s.threshold) {
                PowerSaver.setThreshold(ctx, it)
            }
        }
        SwitchRow(stringResource(R.string.save_battery_mobile_data), s.mobileData) { PowerSaver.setMobileData(ctx, it) }
        Muted(stringResource(R.string.save_battery_mobile_data_hint))
        Muted(saveBatteryLine(PowerSaver.plan, s), Modifier.announce())
    }
}

/** A row of buttons, the chosen one filled (and said "selected"). */
@Composable
private fun <T> Choices(options: List<Pair<T, String>>, chosen: T, pick: (T) -> Unit) {
    Row(Modifier.fillMaxWidth().selectableGroup(), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
        options.forEach { (value, label) ->
            if (value == chosen) PrimaryButton(label, { pick(value) }, Modifier.weight(1f), selected = true)
            else SoftButton(label, { pick(value) }, Modifier.weight(1f), selected = false)
        }
    }
}

/** "Paused now: folder sync, photo backup. Why: Battery Saver is on." */
@Composable
fun saveBatteryLine(plan: PowerPolicy.Plan, s: PowerPolicy.Settings): String {
    if (plan.paused.isEmpty()) return stringResource(R.string.save_battery_status_none)
    val what = PowerPolicy.Paused.entries.filter { plan.pauses(it) }.map {
        stringResource(
            when (it) {
                PowerPolicy.Paused.FOLDER_SYNC -> R.string.save_battery_paused_folder_sync
                PowerPolicy.Paused.BIG_SYNC_FILES -> R.string.save_battery_paused_big_files
                PowerPolicy.Paused.PHOTO_BACKUP -> R.string.save_battery_paused_backup
                PowerPolicy.Paused.CLIP_PICTURES -> R.string.save_battery_paused_clip_pictures
                PowerPolicy.Paused.AUTO_RESTARTS -> R.string.save_battery_paused_auto_restarts
            },
        )
    }
    val why = plan.reasons.map {
        when (it) {
            PowerPolicy.Reason.ALWAYS -> stringResource(R.string.save_battery_why_always)
            PowerPolicy.Reason.BATTERY_SAVER -> stringResource(R.string.save_battery_why_saver)
            PowerPolicy.Reason.LOW_BATTERY -> stringResource(R.string.save_battery_why_low, s.threshold)
            PowerPolicy.Reason.IDLE -> stringResource(R.string.save_battery_why_idle)
            PowerPolicy.Reason.METERED -> stringResource(R.string.save_battery_why_metered)
        }
    }
    return stringResource(R.string.save_battery_status, what.joinToString(", "), why.joinToString(", "))
}

/** On the Home screen while saving battery; a tap opens the settings. */
@Composable
fun SavingBatteryBanner(onOpen: () -> Unit) {
    if (!PowerSaver.plan.saving) return
    Tile(title = stringResource(R.string.save_battery_title), onClick = onOpen) {
        Muted(stringResource(R.string.home_saving_battery), Modifier.announce())
    }
}
