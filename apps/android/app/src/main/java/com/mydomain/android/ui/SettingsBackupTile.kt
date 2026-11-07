package com.mydomain.android.ui

import android.content.Context
import android.content.Intent
import android.net.Uri
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.selection.toggleable
import androidx.compose.material3.Checkbox
import androidx.compose.material3.CheckboxDefaults
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.unit.dp
import com.mydomain.android.R
import com.mydomain.android.core.Core
import com.mydomain.android.core.SettingsBackup
import com.mydomain.android.core.friendlyError
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import java.io.File
import java.text.DateFormat
import java.util.Date

/**
 * Settings → "Back up settings" / "Restore settings…": the phone's side of
 * [SettingsBackup]. Saves through the system's file picker (Storage Access
 * Framework: ACTION_CREATE_DOCUMENT), restores from one it opens
 * (ACTION_OPEN_DOCUMENT) after showing what each section would change, a
 * box per section (sections naming devices that aren't yours here start
 * unticked). Most settings are read at start, so it offers a restart.
 */
private object SettingsBackupIo {
    private const val AUTOMATIONS_FILE = "automations.json"
    /** Sections read once at start: restored on disk, shown after a restart. */
    private val NEEDS_RESTART = setOf("appearance", "preferences", "automations", "device_labels", "page_layout")

    private fun prefs(ctx: Context) = ctx.applicationContext.getSharedPreferences("md", Context.MODE_PRIVATE)

    /** The phone's settings now, as a backup. */
    fun collect(ctx: Context): SettingsBackup.Backup {
        val me = Core.me?.nodeId
        val engine = Core.engine()
        val sharing = if (engine == null) null else {
            val b = Core.batteryAlerts()
            linkedMapOf<String, Any?>(
                "labels_share" to Core.labelsShare(),
                "battery_alerts" to linkedMapOf(
                    "enabled" to (b?.enabled ?: true),
                    "low" to (b?.low?.toLong() ?: 15L),
                    "charged" to (b?.charged?.toLong() ?: 0L),
                ),
            )
        }
        val photo = Core.backup?.let {
            linkedMapOf<String, Any?>("enabled" to it.enabled, "target" to it.target, "charging_only" to it.chargingOnly)
        }
        val pairs = if (engine == null) null else {
            Core.syncPairs.filter { it.path.isNotEmpty() }.map {
                linkedMapOf<String, Any?>("peer" to it.peer, "name" to it.name, "path" to it.path, "mode" to it.mode)
            }
        }
        val automations = File(ctx.applicationContext.filesDir, AUTOMATIONS_FILE).takeIf { it.exists() }
            ?.let { runCatching { it.readText() }.getOrNull() }
        return SettingsBackup.build(
            prefs = prefs(ctx).all,
            automations = automations,
            sharing = sharing,
            photoBackup = photo,
            syncPairs = pairs,
            devices = Core.peers.filter { it.nodeId != me }.map { SettingsBackup.Device(it.nodeId, Core.peerName(it.nodeId)) },
            createdAt = System.currentTimeMillis() / 1000,
        )
    }

    /** Your devices here, this phone too. */
    fun known(): Set<String> = (Core.peers.map { it.nodeId } + listOfNotNull(Core.me?.nodeId)).toSet()

    fun write(ctx: Context, uri: Uri, text: String) {
        val out = ctx.contentResolver.openOutputStream(uri, "wt") ?: error("no stream")
        out.use { it.write(text.toByteArray(Charsets.UTF_8)) }
    }

    /** The file's text; [SettingsBackup.Refused] (too big) past the limit. */
    fun read(ctx: Context, uri: Uri): String {
        val input = ctx.contentResolver.openInputStream(uri) ?: error("no stream")
        val bytes = input.use { s ->
            val buf = java.io.ByteArrayOutputStream()
            val chunk = ByteArray(64 * 1024)
            while (true) {
                val n = s.read(chunk)
                if (n < 0) break
                buf.write(chunk, 0, n)
                if (buf.size() > SettingsBackup.MAX_BYTES) throw SettingsBackup.Refused(SettingsBackup.Problem.TOO_BIG)
            }
            buf.toByteArray()
        }
        return String(bytes, Charsets.UTF_8)
    }

    class Result(val failed: List<String>, val restart: Boolean)

    /** Restore sections [chosen] of [b]. */
    suspend fun apply(ctx: Context, b: SettingsBackup.Backup, chosen: Set<String>): Result {
        val failed = ArrayList<String>()
        val edit = prefs(ctx).edit()
        for ((key, value) in SettingsBackup.prefsToWrite(b, chosen)) {
            when (value) {
                null -> edit.remove(key)
                is Boolean -> edit.putBoolean(key, value)
                is String -> edit.putString(key, value)
                is List<*> -> edit.putStringSet(key, value.filterIsInstance<String>().toSet())
            }
        }
        withContext(Dispatchers.IO) { edit.commit() }
        // The core keeps its own copy of these.
        com.mydomain.android.NearbyMode.apply(ctx)
        com.mydomain.android.PhoneWallpaper.report(ctx)
        if ("automations" in chosen) {
            val text = b.sections["automations"]?.get("automations") as? String
            if (text != null) {
                runCatching {
                    withContext(Dispatchers.IO) {
                        val f = File(ctx.applicationContext.filesDir, AUTOMATIONS_FILE)
                        val tmp = File(f.parentFile, "$AUTOMATIONS_FILE.tmp")
                        tmp.writeText(text)
                        if (!tmp.renameTo(f)) error("rename")
                    }
                }.onFailure { failed.add("${ctx.getString(R.string.settings_backup_section_automations)}: ${friendlyError(it)}") }
            }
        }
        val sharing = b.sections["sharing"]
        if ("sharing" in chosen && sharing != null) {
            Core.setLabelsShare(sharing["labels_share"] as? Boolean ?: true)
            val alerts = sharing["battery_alerts"] as? Map<*, *>
            if (alerts != null) {
                runCatching {
                    Core.setBatteryAlerts(
                        alerts["enabled"] as? Boolean ?: true,
                        (alerts["low"] as? Long)?.toInt() ?: 15,
                        (alerts["charged"] as? Long)?.toInt() ?: 0,
                    )
                }.onFailure { failed.add("${ctx.getString(R.string.settings_backup_section_sharing)}: ${friendlyError(it)}") }
            }
        }
        val photo = b.sections["photo_backup"]
        if ("photo_backup" in chosen && photo != null) {
            runCatching {
                Core.setBackup(photo["enabled"] as? Boolean ?: false, photo["target"] as? String ?: "", photo["charging_only"] as? Boolean ?: false)
            }.onFailure { failed.add("${ctx.getString(R.string.settings_backup_section_photos)}: ${friendlyError(it)}") }
        }
        if ("sync_folders" in chosen) {
            val now = collect(ctx).sections["sync_folders"]
            val known = known()
            for (p in SettingsBackup.newPairs(b.sections["sync_folders"], now)) {
                val name = p["name"] as? String ?: ""
                val peer = p["peer"] as? String ?: continue
                if (peer !in known) {
                    failed.add(ctx.getString(R.string.settings_backup_not_yours, name))
                    continue
                }
                runCatching { Core.call { it.syncCreate(peer, p["path"] as String, p["mode"] as String) } }
                    .onFailure { failed.add("$name: ${friendlyError(it)}") }
            }
            Core.refreshSync()
        }
        return Result(failed, chosen.any { it in NEEDS_RESTART })
    }

    /** Start the app afresh: every module reads what was restored. */
    fun restart(ctx: Context) {
        val intent = ctx.packageManager.getLaunchIntentForPackage(ctx.packageName) ?: return
        intent.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_CLEAR_TASK)
        ctx.startActivity(intent)
        Runtime.getRuntime().exit(0)
    }
}

@Composable
private fun problemText(p: SettingsBackup.Problem): String = stringResource(
    when (p) {
        SettingsBackup.Problem.TOO_BIG -> R.string.settings_backup_err_too_big
        SettingsBackup.Problem.NOT_JSON -> R.string.settings_backup_err_not_json
        SettingsBackup.Problem.NOT_BACKUP -> R.string.settings_backup_err_not_backup
        SettingsBackup.Problem.NEWER -> R.string.settings_backup_err_newer
        SettingsBackup.Problem.UNKNOWN_VERSION -> R.string.settings_backup_err_version
        SettingsBackup.Problem.FROM_COMPUTER -> R.string.settings_backup_err_computer
        SettingsBackup.Problem.DAMAGED -> R.string.settings_backup_err_damaged
    },
)

private fun sectionTitle(id: String): Int = when (id) {
    "appearance" -> R.string.settings_backup_section_appearance
    "preferences" -> R.string.settings_backup_section_preferences
    "sharing" -> R.string.settings_backup_section_sharing
    "automations" -> R.string.settings_backup_section_automations
    "device_labels" -> R.string.settings_backup_section_labels
    "page_layout" -> R.string.settings_backup_section_layout
    "photo_backup" -> R.string.settings_backup_section_photos
    else -> R.string.settings_backup_section_sync
}

@Composable
fun SettingsBackupTile(modifier: Modifier = Modifier) {
    val ctx = LocalContext.current
    val toast = LocalToast.current
    val scope = rememberCoroutineScope()
    var busy by remember { mutableStateOf(false) }
    var pending by remember { mutableStateOf<SettingsBackup.Backup?>(null) }
    var preview by remember { mutableStateOf<List<SettingsBackup.SectionPreview>>(emptyList()) }
    var picked by remember { mutableStateOf<Set<String>>(emptySet()) }
    var problem by remember { mutableStateOf<SettingsBackup.Problem?>(null) }
    var failed by remember { mutableStateOf<List<String>>(emptyList()) }
    var askRestart by remember { mutableStateOf(false) }
    val saved = stringResource(R.string.settings_backup_saved)
    val restored = stringResource(R.string.settings_backup_restored)

    val create = rememberLauncherForActivityResult(ActivityResultContracts.CreateDocument("application/json")) { uri ->
        if (uri == null) return@rememberLauncherForActivityResult
        busy = true
        scope.launch {
            try {
                val text = SettingsBackup.encode(SettingsBackupIo.collect(ctx))
                withContext(Dispatchers.IO) { SettingsBackupIo.write(ctx, uri, text) }
                toast(saved)
            } catch (e: Exception) {
                toast(friendlyError(e))
            } finally {
                busy = false
            }
        }
    }
    val open = rememberLauncherForActivityResult(ActivityResultContracts.OpenDocument()) { uri ->
        if (uri == null) return@rememberLauncherForActivityResult
        busy = true
        problem = null
        failed = emptyList()
        askRestart = false
        scope.launch {
            try {
                val text = withContext(Dispatchers.IO) { SettingsBackupIo.read(ctx, uri) }
                val b = SettingsBackup.parse(text)
                val p = SettingsBackup.preview(SettingsBackupIo.collect(ctx), b, SettingsBackupIo.known())
                pending = b
                preview = p
                picked = p.filter { it.on && it.changed }.map { it.id }.toSet()
            } catch (e: SettingsBackup.Refused) {
                problem = e.problem
            } catch (e: Exception) {
                toast(friendlyError(e))
            } finally {
                busy = false
            }
        }
    }

    Tile(modifier, title = stringResource(R.string.settings_backup_title)) {
        Muted(stringResource(R.string.settings_restore_hint))
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            PrimaryButton(stringResource(R.string.settings_backup_save), {
                val day = java.text.SimpleDateFormat("yyyy-MM-dd", java.util.Locale.ROOT).format(Date())
                create.launch("my-domain settings $day.json")
            }, enabled = !busy)
            SoftButton(stringResource(R.string.settings_backup_restore), {
                open.launch(arrayOf("application/json", "text/plain", "application/octet-stream"))
            }, enabled = !busy)
        }
        Muted(stringResource(R.string.settings_backup_never))
        problem?.let { Text(problemText(it), style = MaterialTheme.typography.bodySmall, color = T.warning) }

        val b = pending
        if (b != null) {
            Text(stringResource(R.string.settings_backup_preview_title), style = MaterialTheme.typography.titleSmall, color = T.text)
            val made = DateFormat.getDateTimeInstance(DateFormat.MEDIUM, DateFormat.SHORT).format(Date(b.createdAt * 1000))
            Muted(stringResource(R.string.settings_backup_from, made))
            Column {
                for (s in preview) {
                    Row(
                        Modifier.fillMaxWidth().heightIn(min = 48.dp).toggleable(
                            value = s.id in picked, enabled = s.changed, role = Role.Checkbox,
                            onValueChange = { on -> picked = if (on) picked + s.id else picked - s.id },
                        ),
                        verticalAlignment = Alignment.CenterVertically,
                        horizontalArrangement = Arrangement.spacedBy(8.dp),
                    ) {
                        Checkbox(
                            checked = s.id in picked, onCheckedChange = null, enabled = s.changed,
                            colors = CheckboxDefaults.colors(checkedColor = T.accent, uncheckedColor = T.controlBorder),
                        )
                        Column(Modifier.weight(1f)) {
                            Text(stringResource(sectionTitle(s.id)), style = MaterialTheme.typography.bodyMedium, color = T.text)
                            Muted(
                                if (s.changed) stringResource(R.string.settings_backup_will_change)
                                else stringResource(R.string.settings_backup_same),
                            )
                            if (s.unknownDevices.isNotEmpty()) {
                                Text(
                                    stringResource(R.string.settings_backup_unknown_devices, s.unknownDevices.joinToString(", ")),
                                    style = MaterialTheme.typography.bodySmall, color = T.warning,
                                )
                            }
                        }
                    }
                }
            }
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                PrimaryButton(stringResource(R.string.settings_backup_apply), {
                    busy = true
                    scope.launch {
                        try {
                            val r = SettingsBackupIo.apply(ctx, b, picked)
                            pending = null
                            failed = r.failed
                            askRestart = r.restart
                            if (r.failed.isEmpty()) toast(restored)
                        } catch (e: Exception) {
                            toast(friendlyError(e))
                        } finally {
                            busy = false
                        }
                    }
                }, enabled = !busy && picked.isNotEmpty())
                GhostButton(stringResource(R.string.cancel), { pending = null })
            }
        }

        if (failed.isNotEmpty()) {
            Text(stringResource(R.string.settings_backup_some_failed), style = MaterialTheme.typography.bodySmall, color = T.warning)
            for (f in failed) Muted(f)
        }
        if (askRestart) {
            Muted(stringResource(R.string.settings_backup_restart_hint))
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                PrimaryButton(stringResource(R.string.settings_backup_restart_now), { SettingsBackupIo.restart(ctx) })
                GhostButton(stringResource(R.string.settings_backup_later), { askRestart = false })
            }
        }
    }
}
