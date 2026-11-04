package com.mydomain.android.ui

import androidx.compose.foundation.selection.selectableGroup
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.heading
import androidx.activity.compose.BackHandler
import androidx.compose.foundation.horizontalScroll
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
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import com.mydomain.android.AutoPresets
import com.mydomain.android.PhoneAutomations
import com.mydomain.android.QuickTarget
import com.mydomain.android.R
import com.mydomain.android.core.Automation
import com.mydomain.android.core.AutomationBook
import com.mydomain.android.core.Core
import com.mydomain.android.core.Trigger
import com.mydomain.android.core.TriggerKind
import com.mydomain.android.core.timeAgo
import org.json.JSONObject
import java.util.UUID

/** What can start one, with how the form says it. */
private val KINDS = listOf(
    TriggerKind.CONNECTS to R.string.automation_kind_connects,
    TriggerKind.DISCONNECTS to R.string.automation_kind_disconnects,
    TriggerKind.BATTERY_BELOW to R.string.automation_kind_battery_below,
    TriggerKind.CHARGING_STARTS to R.string.automation_kind_charging_starts,
    TriggerKind.CHARGING_STOPS to R.string.automation_kind_charging_stops,
    TriggerKind.WIFI_JOINS to R.string.automation_kind_wifi_joins,
    TriggerKind.WIFI_LEAVES to R.string.automation_kind_wifi_leaves,
)

/**
 * The phone's automations (pushed from Settings): the list (what starts
 * each, what it does, on/off, edit, delete) and the form that adds or
 * changes one. They live and run in [PhoneAutomations].
 */
@Composable
fun AutomationsScreen(onBack: () -> Unit) {
    val ctx = LocalContext.current
    val toast = LocalToast.current
    remember { PhoneAutomations.load(ctx) }
    /** The one being edited (a fresh one when [isNew]), or none. */
    var editing by remember { mutableStateOf<Automation?>(null) }
    var isNew by remember { mutableStateOf(false) }
    val deleted = stringResource(R.string.automation_deleted)

    BackHandler(enabled = editing != null) { editing = null }

    Column(
        Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(start = 16.dp, end = 16.dp, bottom = 16.dp),
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        Row(Modifier.padding(top = 4.dp), verticalAlignment = Alignment.CenterVertically) {
            IconButton(onClick = { if (editing != null) editing = null else onBack() }) {
                Icon(Icons.AutoMirrored.Filled.ArrowBack, stringResource(R.string.back), tint = T.text)
            }
            Text(
                stringResource(R.string.automations_title), Modifier.semantics { heading() },
                style = MaterialTheme.typography.headlineSmall, color = T.text,
            )
        }
        val e = editing
        if (e != null) {
            AutomationForm(e, isNew, onDone = { editing = null })
        } else {
            Muted(stringResource(R.string.automations_hint))
            if (PhoneAutomations.items.isEmpty()) {
                EmptyState(stringResource(R.string.automations_empty_title), stringResource(R.string.automations_empty_body))
            }
            PhoneAutomations.items.forEach { a ->
                AutomationRow(
                    a,
                    onEdit = { isNew = false; editing = a },
                    onDelete = { PhoneAutomations.delete(ctx, a.id); toast(deleted) },
                )
            }
            PrimaryButton(stringResource(R.string.automation_add), {
                isNew = true
                editing = draft(QuickTarget.computer(ctx) ?: otherPeers().firstOrNull() ?: "")
            }, Modifier.fillMaxWidth())
        }
    }
}

/** Your other devices' node ids. */
private fun otherPeers(): List<String> {
    val me = Core.me?.nodeId
    return Core.peers.map { it.nodeId }.filter { it != me }
}

/** A new automation: "when [computer] connects, tell me". */
private fun draft(computer: String) = Automation(
    id = UUID.randomUUID().toString(),
    trigger = Trigger(TriggerKind.CONNECTS, node = computer),
    node = "",
    action = AutomationBook.NOTIFY,
)

@Composable
private fun AutomationRow(a: Automation, onEdit: () -> Unit, onDelete: () -> Unit) {
    val ctx = LocalContext.current
    Tile {
        SwitchRow(
            stringResource(R.string.automation_arrow, PhoneAutomations.whenText(ctx, a.trigger), PhoneAutomations.thenText(ctx, a)),
            a.enabled,
        ) { on -> PhoneAutomations.setEnabled(ctx, a.id, on) }
        Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            Muted(
                a.lastFired?.let { stringResource(R.string.automation_last_fired, timeAgo(it)) }
                    ?: stringResource(R.string.automation_never_fired),
                Modifier.weight(1f),
            )
            GhostButton(stringResource(R.string.automation_edit), onEdit, small = true)
            GhostButton(stringResource(R.string.automation_delete), onDelete, small = true)
        }
    }
}

/** A row of choices, the chosen one highlighted; scrolls sideways. */
@Composable
private fun <V> Choices(options: List<Pair<V, String>>, selected: V, onPick: (V) -> Unit) {
    Row(
        Modifier.fillMaxWidth().selectableGroup().horizontalScroll(rememberScrollState()),
        horizontalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        options.forEach { (v, label) ->
            // The chosen one is filled, and says "selected".
            if (v == selected) PrimaryButton(label, { onPick(v) }, selected = true)
            else SoftButton(label, { onPick(v) }, selected = false)
        }
    }
}

/** Adds ([isNew]) or changes [start]. */
@Composable
private fun AutomationForm(start: Automation, isNew: Boolean, onDone: () -> Unit) {
    val ctx = LocalContext.current
    val toast = LocalToast.current
    var kind by remember(start.id) { mutableStateOf(start.trigger.kind) }
    var watch by remember(start.id) { mutableStateOf(start.trigger.node) }
    var level by remember(start.id) { mutableStateOf((start.trigger.level ?: 20).toString()) }
    var ssid by remember(start.id) { mutableStateOf(start.trigger.ssid) }
    var target by remember(start.id) { mutableStateOf(start.node) }
    var presetId by remember(start.id) { mutableStateOf(AutoPresets.of(start)?.id ?: "") }
    var note by remember(start.id) {
        mutableStateOf(
            if (start.action == AutoPresets.SEND_TEXT) runCatching { JSONObject(start.args).optString("text") }.getOrDefault("") else "",
        )
    }
    val canName = remember { PhoneAutomations.canSeeWifiName(ctx) }
    val peers = otherPeers()
    val added = stringResource(R.string.automation_added)
    val saved = stringResource(R.string.automation_saved)
    val badLevel = stringResource(R.string.automation_bad_level)
    val pickDevice = stringResource(R.string.automation_pick_device)
    val emptyNote = stringResource(R.string.automation_empty_note)

    // The one it does now stays offered while that device's status is unknown.
    val presets = AutoPresets.offered(target).let { list ->
        val own = AutoPresets.of(start)
        if (own != null && target == start.node && list.none { it.id == own.id }) list + own else list
    }
    val preset = presets.firstOrNull { it.id == presetId } ?: presets.firstOrNull()

    Tile(title = stringResource(if (isNew) R.string.automation_new else R.string.automation_edit_title)) {
        Muted(stringResource(R.string.automation_when_label))
        Choices(KINDS.map { (k, label) -> k to stringResource(label) }, kind) { kind = it }

        if (TriggerKind.watchesDevice(kind)) {
            Muted(stringResource(R.string.automation_watch_label))
            if (peers.isEmpty()) {
                Muted(stringResource(R.string.automation_no_devices))
            } else {
                Choices(peers.map { it to Core.peerName(it) }, watch) { watch = it }
            }
        }
        if (kind == TriggerKind.BATTERY_BELOW) {
            Field(level, { v -> level = v.filter(Char::isDigit).take(2) }, stringResource(R.string.automation_level_label))
        }
        if (TriggerKind.isWifi(kind)) {
            if (canName) {
                Field(ssid, { ssid = it.take(AutomationBook.MAX_SSID) }, stringResource(R.string.automation_ssid_label))
                val current = remember { PhoneAutomations.currentSsid(ctx) }
                if (current != null && current != ssid) {
                    GhostButton(stringResource(R.string.automation_ssid_use, current), { ssid = current }, small = true)
                }
            } else {
                Muted(stringResource(R.string.automation_wifi_any_only))
            }
        }

        Muted(stringResource(R.string.automation_target_label))
        Choices(
            listOf("" to stringResource(R.string.automation_this_phone_option)) + peers.map { it to Core.peerName(it) },
            target,
        ) { target = it }

        Muted(stringResource(R.string.automation_action_label))
        if (preset != null) {
            Choices(presets.map { it.id to stringResource(it.label) }, preset.id) { presetId = it }
        }
        if (preset?.action == AutoPresets.SEND_TEXT) {
            Field(note, { note = it.take(500) }, stringResource(R.string.automation_note_label), singleLine = false)
        }
        if (target.isNotEmpty() && Core.deviceStatus[target] == null) {
            Muted(stringResource(R.string.automation_more_when_connected, Core.peerName(target)))
        }

        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            PrimaryButton(stringResource(R.string.save), save@{
                val p = preset ?: return@save
                val lvl = level.toIntOrNull()
                when {
                    TriggerKind.watchesDevice(kind) && watch.isEmpty() -> { toast(pickDevice); return@save }
                    kind == TriggerKind.BATTERY_BELOW && (lvl == null || lvl !in 1..99) -> { toast(badLevel); return@save }
                    p.action == AutoPresets.SEND_TEXT && note.isBlank() -> { toast(emptyNote); return@save }
                }
                val a = start.copy(
                    trigger = Trigger(
                        kind = kind,
                        node = if (TriggerKind.watchesDevice(kind)) watch else "",
                        level = if (kind == TriggerKind.BATTERY_BELOW) lvl else null,
                        // Without location access, only "any Wi-Fi".
                        ssid = if (TriggerKind.isWifi(kind) && canName) ssid.trim() else "",
                    ),
                    node = target,
                    action = p.action,
                    args = if (p.action == AutoPresets.SEND_TEXT) JSONObject().put("text", note.trim()).toString() else p.args,
                    label = ctx.getString(p.label),
                )
                val err = if (isNew) PhoneAutomations.add(ctx, a) else PhoneAutomations.update(ctx, a)
                if (err != null) {
                    toast(err)
                } else {
                    toast(if (isNew) added else saved)
                    onDone()
                }
            }, enabled = preset != null)
            GhostButton(stringResource(R.string.cancel), onDone)
        }
    }
}
