package com.mydomain.android.ui

import androidx.compose.ui.res.pluralStringResource
import android.app.DownloadManager
import android.content.Context
import android.content.Intent
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
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
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import com.mydomain.android.PhoneLocation
import com.mydomain.android.R
import com.mydomain.android.core.Core
import com.mydomain.android.core.DeviceKinds
import com.mydomain.android.core.Health
import com.mydomain.android.core.HealthPlan
import com.mydomain.android.core.SetupItem
import com.mydomain.android.core.SetupStatus
import com.mydomain.android.core.friendlyError
import com.mydomain.android.core.timeAgo
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch

/** Where a health check's buttons go. */
class HealthNav(
    val onDevice: (String) -> Unit = {},
    val onMap: () -> Unit = {},
    val onSettings: () -> Unit = {},
    val onHelp: (String) -> Unit = {},
)

// --- words ---------------------------------------------------------------------

private fun featureRes(f: String): Int? = when (f) {
    "ping" -> R.string.health_feature_ping
    "links_map" -> R.string.health_feature_links_map
    "nearby" -> R.string.health_feature_nearby
    "camera_view" -> R.string.health_feature_camera_view
    "timers" -> R.string.health_feature_timers
    "notes" -> R.string.health_feature_notes
    "read_later" -> R.string.health_feature_read_later
    "insert_capture" -> R.string.health_feature_insert_capture
    "remote_download" -> R.string.health_feature_remote_download
    "notif_rich" -> R.string.health_feature_notif_rich
    "xfer" -> R.string.health_feature_xfer
    "media_ctl" -> R.string.health_feature_media_ctl
    else -> groupRes(f)
}

private fun groupRes(g: String): Int? = when (g) {
    "clipboard" -> R.string.onboard_group_clipboard
    "notifications" -> R.string.onboard_group_notifications
    "files" -> R.string.onboard_group_files
    "media" -> R.string.onboard_group_media
    "find" -> R.string.onboard_group_find
    else -> null
}

private fun whyGroupRes(g: String): Int = when (g) {
    "clipboard" -> R.string.onboard_why_clipboard
    "notifications" -> R.string.onboard_why_notifications
    "files" -> R.string.onboard_why_files
    "media" -> R.string.onboard_why_media
    else -> R.string.onboard_why_find
}

private fun stepRes(step: String): Int = when (step) {
    "name" -> R.string.onboard_step_name
    "devices" -> R.string.onboard_step_devices
    "features" -> R.string.onboard_step_features
    "permissions" -> R.string.onboard_step_permissions
    else -> R.string.onboard_step_connection
}

private fun fixRes(fix: String): Int = when (fix) {
    "retry" -> R.string.health_fix_retry
    "push_test" -> R.string.health_fix_push_test
    "open_permission" -> R.string.health_fix_open_permission
    "free_space" -> R.string.health_fix_free_space
    "trust_key" -> R.string.health_fix_trust_key
    "encryption_key" -> R.string.health_fix_encryption_key
    "connect" -> R.string.health_fix_connect
    "links_map" -> R.string.health_fix_links_map
    "permissions" -> R.string.health_fix_permissions
    else -> R.string.health_fix_update
}

@Composable
private fun features(list: List<String>): String =
    list.map { f -> featureRes(f)?.let { stringResource(it) } ?: f }.joinToString(", ")

@Composable
private fun megabytes(mb: String?): String {
    val n = mb?.toLongOrNull() ?: 0
    return if (n >= 1024) String.format(java.util.Locale.ROOT, "%.1f GB", n / 1024.0) else "$n MB"
}

/** The line's title. */
@Composable
fun checkTitle(c: HealthPlan.Check): String {
    val name = c.nodeId?.let { Core.peerName(it) } ?: ""
    return when (c.id) {
        "server" -> stringResource(if (c.reason == "ok") R.string.health_server_ok else R.string.health_server_unreachable)
        "push" -> stringResource(
            when (c.reason) {
                "working" -> R.string.health_push_working
                "no_token" -> R.string.health_push_no_token
                else -> R.string.health_push_unconfirmed
            },
        )
        "disk" -> stringResource(
            when (c.reason) {
                "full" -> R.string.health_disk_full
                "low" -> R.string.health_disk_low
                else -> R.string.health_disk_ok
            },
            megabytes(c.detail),
        )
        "permission" -> HealthPlan.itemOf(c.detail)?.let { stringResource(titleOf(it)) } ?: (c.detail ?: "")
        "key" -> stringResource(R.string.health_key_changed, name)
        "link" -> when (c.reason) {
            "direct" -> stringResource(R.string.health_link_direct, name, stringResource(howRes(c.detail)))
            "relay" -> stringResource(R.string.health_link_relay, name)
            "ble" -> stringResource(R.string.health_link_ble, name)
            "unreachable" -> stringResource(R.string.health_link_unreachable, name)
            "asleep" -> stringResource(R.string.health_link_asleep, name)
            "waking" -> stringResource(R.string.health_link_waking, name)
            "key_changed" -> stringResource(R.string.health_key_changed, name)
            "bad_key" -> stringResource(R.string.health_link_bad_key, name)
            else -> stringResource(R.string.health_link_not_connected, name)
        }
        "version" -> when {
            c.reason == "same" -> stringResource(R.string.health_version_same, name)
            c.reason == "newer" -> stringResource(R.string.health_version_newer, name, c.detail ?: "")
            c.detail == null -> stringResource(R.string.health_version_older_unknown, name)
            else -> stringResource(R.string.health_version_older, name, c.detail)
        }
        "allowed" -> stringResource(R.string.health_allowed, name)
        else -> c.id
    }
}

/** What to know, under the title ("" for nothing). */
@Composable
fun checkDetail(c: HealthPlan.Check): String {
    val name = c.nodeId?.let { Core.peerName(it) } ?: ""
    return when (c.id) {
        "server" -> if (c.reason == "ok") "" else stringResource(R.string.health_server_unreachable_hint)
        "push" -> when (c.reason) {
            "no_token" -> stringResource(R.string.health_push_no_token_hint)
            "unconfirmed" -> stringResource(R.string.health_push_unconfirmed_hint)
            else -> ""
        }
        "disk" -> if (c.reason == "ok") "" else stringResource(R.string.health_disk_hint)
        "permission" -> {
            val state = stringResource(
                when (c.reason) {
                    "granted" -> R.string.health_perm_granted
                    "denied" -> R.string.health_perm_denied
                    else -> R.string.health_perm_unknown
                },
            )
            if (c.features.isEmpty()) state else state + " · " + stringResource(R.string.health_perm_for, features(c.features))
        }
        "key" -> stringResource(R.string.health_key_hint)
        "link" -> {
            val parts = mutableListOf<String>()
            if (c.reason == "bad_key") parts += stringResource(R.string.health_link_bad_key_hint)
            val d = c.detail
            if (d != null && c.reason != "direct") {
                val why = if (':' in d) stringResource(whyRes(d.replace(':', '_')).first, name)
                else reasonRes(d)?.let { stringResource(it, name) }
                if (why != null) parts += stringResource(R.string.health_link_why, why)
            }
            c.reconnectMs?.let { parts += stringResource(R.string.health_link_reconnect, HealthPlan.millis(it)) }
            parts.joinToString(" · ")
        }
        "version" -> when {
            c.reason == "older" && c.features.isNotEmpty() -> stringResource(R.string.health_version_older_hint, features(c.features))
            c.reason == "newer" -> stringResource(R.string.health_version_newer_hint)
            else -> ""
        }
        "allowed" -> c.features.map { stringResource(permRes(it)) }.joinToString(", ")
        else -> ""
    }
}

/** What a device may do here, in words (the health check's "allowed"). */
private fun permRes(perm: String): Int = when (perm) {
    "control" -> R.string.perm_name_control
    "kvm" -> R.string.perm_name_kvm
    else -> R.string.perm_name_screen
}

// --- fixing ----------------------------------------------------------------------

/** What a check's button does. Permissions open Android's own prompt or
 *  page (never turned on from here). */
@Composable
fun rememberHealthFix(nav: HealthNav): (HealthPlan.Check) -> Unit {
    val ctx = LocalContext.current
    val toast = LocalToast.current
    val scope = rememberCoroutineScope()
    val sent = stringResource(R.string.health_push_sent)
    val turnOn = rememberTurnOn { scope.launch { Health.run(ctx, server = false) } }
    return { c ->
        when (c.fix) {
            "retry" -> scope.launch { Health.run(ctx, server = true) }
            "push_test" -> scope.launch {
                try {
                    Health.pushTest()
                    toast(sent)
                    delay(8_000)
                    Health.run(ctx, server = false)
                } catch (e: Exception) {
                    toast(friendlyError(e))
                }
            }
            "open_permission" -> HealthPlan.itemOf(c.detail)?.let(turnOn)
            "free_space" -> runCatching {
                ctx.startActivity(Intent(DownloadManager.ACTION_VIEW_DOWNLOADS).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
            }
            "trust_key", "permissions" -> c.nodeId?.let(nav.onDevice)
            "encryption_key" -> nav.onSettings()
            "connect" -> c.nodeId?.let { node ->
                scope.launch {
                    try {
                        Core.connect(node)
                    } catch (e: Exception) {
                        toast(friendlyError(e))
                    }
                    delay(3_000)
                    Health.run(ctx, server = false)
                }
            }
            "links_map" -> nav.onMap()
            "update_other", "update_app" -> nav.onHelp("troubleshooting")
        }
    }
}

/** One line: a dot for the level, what it is, why, and its fix. */
@Composable
fun HealthRow(c: HealthPlan.Check, fix: (HealthPlan.Check) -> Unit) {
    val color = when (c.level) {
        "ok" -> T.success
        "warn" -> T.warning
        "bad" -> T.danger
        else -> T.text3
    }
    val level = stringResource(
        when (c.level) {
            "ok" -> R.string.health_level_ok
            "warn" -> R.string.health_level_warn
            "bad" -> R.string.health_level_bad
            else -> R.string.health_level_info
        },
    )
    Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
        val dot = Modifier.size(10.dp).semantics { contentDescription = level }
        Box(
            if (c.level == "info") dot.border(1.5.dp, color, CircleShape) else dot.background(color, CircleShape),
        )
        Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(2.dp)) {
            Text(checkTitle(c), style = MaterialTheme.typography.bodyLarge, fontWeight = FontWeight.SemiBold, color = T.text)
            checkDetail(c).takeIf { it.isNotEmpty() }?.let { Muted(it) }
        }
        if (HealthPlan.showsFix(c)) SoftButton(stringResource(fixRes(c.fix!!)), { fix(c) })
    }
}

/** Settings → Health: every check, worst first, with "Run again". */
@Composable
fun HealthScreen(onBack: () -> Unit, nav: HealthNav) {
    val ctx = LocalContext.current
    val scope = rememberCoroutineScope()
    val fix = rememberHealthFix(nav)
    // Back from Android's Settings: the checklist is read again, and so is this.
    val rows = rememberSetupRows()
    LaunchedEffect(rows.rows) { Health.run(ctx, server = Health.ranAt == 0L) }
    Column(
        Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(start = 16.dp, end = 16.dp, bottom = 16.dp),
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        BackBar(stringResource(R.string.health_title), onBack)
        if (Health.ranAt > 0) Muted(stringResource(R.string.health_last_run, timeAgo(Health.ranAt)))
        Muted(stringResource(R.string.health_subtitle))
        PrimaryButton(
            stringResource(if (Health.running) R.string.health_running else R.string.health_rerun),
            { scope.launch { Health.run(ctx, server = true) } },
            Modifier.fillMaxWidth(),
            enabled = !Health.running,
        )
        val checks = Health.checks
        val bad = checks.count { it.level == "bad" }
        val warn = checks.count { it.level == "warn" }
        Tile {
            Text(
                when {
                    checks.isEmpty() -> stringResource(R.string.health_running)
                    bad == 0 && warn == 0 -> stringResource(R.string.health_all_good)
                    else -> listOfNotNull(
                        if (bad > 0) pluralStringResource(R.plurals.health_needs_fixing, bad, bad) else null,
                        if (warn > 0) pluralStringResource(R.plurals.health_worth_look, warn, warn) else null,
                    ).joinToString(" · ")
                },
                style = MaterialTheme.typography.titleMedium, color = T.text,
            )
            checks.forEach { HealthRow(it, fix) }
        }
    }
}

/** A device's screen, while the check has something red about it. */
@Composable
fun HealthDeviceBanner(node: String, onOpen: () -> Unit) {
    if (HealthPlan.deviceProblems(Health.checks, node).none { it.bad }) return
    Tile {
        Text(stringResource(R.string.health_device_banner), color = T.danger, style = MaterialTheme.typography.bodyMedium)
        SoftButton(stringResource(R.string.health_open), onOpen)
    }
}

/** Settings' tile: Health check and "Run setup again". */
@Composable
fun HealthSettingsTile(onHealth: () -> Unit, onSetup: () -> Unit) {
    val scope = rememberCoroutineScope()
    Tile(title = stringResource(R.string.health_settings_tile)) {
        Muted(stringResource(R.string.health_settings_hint))
        Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            SoftButton(stringResource(R.string.health_open), onHealth, Modifier.weight(1f))
            GhostButton(stringResource(R.string.onboard_again), {
                scope.launch {
                    runCatching { Health.act("restart") }
                    onSetup()
                }
            }, Modifier.weight(1f))
        }
    }
}

// --- first-run setup -------------------------------------------------------------

/** Home's card while setup isn't finished: carries on where it stopped. */
@Composable
fun SetupResumeCard(onOpen: () -> Unit) {
    val scope = rememberCoroutineScope()
    LaunchedEffect(Unit) { Health.loadSetup() }
    val s = Health.setup ?: return
    if (s.finished) return
    Tile(title = stringResource(R.string.onboard_card_title)) {
        Muted(
            stringResource(
                R.string.onboard_card_body, HealthPlan.stepNumber(s.step), HealthPlan.STEPS.size, stringResource(stepRes(s.step)),
            ),
        )
        Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            PrimaryButton(stringResource(R.string.onboard_card_open), onOpen, Modifier.weight(1f))
            GhostButton(stringResource(R.string.onboard_skip_all), { scope.launch { runCatching { Health.act("skip_all") } } }, Modifier.weight(1f))
        }
    }
}

/** The in-app switches for the groups picked: clipboard sync follows the
 *  pick; location sharing is only ever turned off here (turning it on is
 *  its own switch on the next step). */
private fun applyGroups(ctx: Context, groups: List<String>) {
    val on = "clipboard" in groups
    Core.enableClipSync(on)
    ctx.getSharedPreferences("md", Context.MODE_PRIVATE).edit().putBoolean("clip_sync", on).apply()
    if ("find" !in groups) PhoneLocation.setAllowed(ctx, false)
}

/** First-run setup: name this phone, add your other devices, choose what
 *  to use, allow what that needs (Android's own prompts and pages), test
 *  the connection. Every step can be skipped; "Later" closes it (Home
 *  carries on), "Skip setup" ends it. */
@Composable
fun OnboardingScreen(onClose: () -> Unit, nav: HealthNav) {
    val ctx = LocalContext.current
    val toast = LocalToast.current
    val scope = rememberCoroutineScope()
    val fix = rememberHealthFix(nav)
    val rows = rememberSetupRows()
    val turnOn = rememberTurnOn { rows.refresh() }
    // The name typed and the groups picked survive a rotation (the groups
    // as one comma-separated line; read from the saved setup only once).
    var name by rememberSaveable { mutableStateOf(HealthPlan.suggestedName(Core.me?.name ?: "", DeviceKinds.own?.model ?: "")) }
    var pickedLine by rememberSaveable { mutableStateOf(HealthPlan.GROUPS.joinToString(",")) }
    var pickedLoaded by rememberSaveable { mutableStateOf(false) }
    val picked = pickedLine.split(',').filter { it.isNotEmpty() }.toSet()
    fun pick(next: Set<String>) {
        pickedLine = HealthPlan.GROUPS.filter { it in next }.joinToString(",")
    }
    var busy by remember { mutableStateOf(false) }
    LaunchedEffect(Unit) {
        val s = Health.loadSetup()
        if (s != null && !pickedLoaded) pick(s.enabled.toSet())
        pickedLoaded = true
    }
    val step = Health.setup?.step ?: "name"
    // The connection step is live.
    LaunchedEffect(step) {
        while (step == "connection") {
            Health.run(ctx, server = false)
            delay(4_000)
        }
    }

    val act: (String, String?) -> Unit = { action, to ->
        scope.launch {
            busy = true
            try {
                val s = Health.act(action, to)
                if (s.finished) onClose()
            } catch (e: Exception) {
                toast(friendlyError(e))
            } finally {
                busy = false
            }
        }
    }
    val next: () -> Unit = {
        scope.launch {
            busy = true
            try {
                when (step) {
                    "name" -> Health.setName(name)
                    "features" -> {
                        Health.setFeatures(HealthPlan.GROUPS.filter { it in picked })
                        applyGroups(ctx, picked.toList())
                    }
                }
                val s = Health.act(if (step == "connection") "finish" else "next")
                if (s.finished) onClose()
            } catch (e: Exception) {
                toast(friendlyError(e))
            } finally {
                busy = false
            }
        }
    }

    Column(
        Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(start = 16.dp, end = 16.dp, bottom = 16.dp),
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        Row(Modifier.fillMaxWidth().padding(top = 8.dp), verticalAlignment = Alignment.CenterVertically) {
            Column(Modifier.weight(1f)) {
                Muted(stringResource(R.string.onboard_step_of, HealthPlan.stepNumber(step), HealthPlan.STEPS.size))
                Text(
                    stringResource(stepRes(step)), Modifier.semantics { heading() },
                    style = MaterialTheme.typography.headlineSmall, color = T.text,
                )
            }
            GhostButton(stringResource(R.string.onboard_later), onClose, small = true)
        }
        Tile {
            when (step) {
                "name" -> {
                    Muted(stringResource(R.string.onboard_name_hint))
                    Field(name, { name = it.take(40) }, stringResource(R.string.onboard_name_label))
                }
                "devices" -> {
                    Muted(stringResource(R.string.onboard_devices_hint))
                    AddDevice()
                    if (Core.peers.isEmpty()) Muted(stringResource(R.string.onboard_devices_none))
                    else Muted(stringResource(R.string.onboard_devices_have, Core.peers.joinToString(", ") { Core.peerName(it.nodeId) }))
                }
                "features" -> {
                    Muted(stringResource(R.string.onboard_features_hint))
                    HealthPlan.GROUPS.forEach { g ->
                        SwitchRow(stringResource(groupRes(g)!!), g in picked) { on -> pick(if (on) picked + g else picked - g) }
                        Muted(stringResource(whyGroupRes(g)))
                    }
                }
                "permissions" -> {
                    Muted(stringResource(R.string.onboard_permissions_hint))
                    val shown = HealthPlan.forGroups(rows.rows, picked.toList())
                    if (shown.isEmpty()) Muted(stringResource(R.string.onboard_permissions_none))
                    shown.forEach { row ->
                        Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                            Column(Modifier.weight(1f)) {
                                Text(stringResource(titleOf(row.item)), color = T.text, fontWeight = FontWeight.SemiBold)
                                Muted(stringResource(purposeOf(row.item)))
                            }
                            when {
                                // An in-app choice, not a permission: its switch.
                                row.item == SetupItem.LOCATION_SHARING -> androidx.compose.material3.Switch(
                                    checked = row.status == SetupStatus.ON,
                                    onCheckedChange = { on -> PhoneLocation.setAllowed(ctx, on); rows.refresh() },
                                )
                                row.status == SetupStatus.ON ->
                                    Text(stringResource(R.string.setup_on), color = T.success, fontWeight = FontWeight.Bold)
                                else -> SoftButton(stringResource(R.string.health_fix_open_permission), { turnOn(row.item) })
                            }
                        }
                    }
                }
                else -> {
                    Muted(stringResource(R.string.onboard_connection_hint))
                    val links = Health.checks.filter { it.id == "link" || it.id == "key" }
                    if (links.isEmpty()) Muted(stringResource(R.string.onboard_connection_none))
                    links.forEach { HealthRow(it, fix) }
                }
            }
        }
        Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            if (HealthPlan.stepNumber(step) > 1) GhostButton(stringResource(R.string.onboard_back), { if (!busy) act("back", null) }, Modifier.weight(1f))
            GhostButton(stringResource(R.string.onboard_skip), { if (!busy) act("skip", null) }, Modifier.weight(1f))
            PrimaryButton(
                stringResource(if (step == "connection") R.string.onboard_done else R.string.onboard_next),
                next,
                Modifier.weight(1f),
                enabled = !busy && (step != "name" || name.isNotBlank()),
            )
        }
        GhostButton(stringResource(R.string.onboard_skip_all), { if (!busy) act("skip_all", null) }, Modifier.fillMaxWidth(), small = true)
    }
}
