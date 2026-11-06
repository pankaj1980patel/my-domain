package com.mydomain.android.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.selection.toggleable
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.outlined.Info
import androidx.compose.material3.Checkbox
import androidx.compose.material3.CheckboxDefaults
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.OutlinedTextFieldDefaults
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableLongStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.pluralStringResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.mydomain.android.NoteReminders
import com.mydomain.android.R
import com.mydomain.android.TimerAlarms
import com.mydomain.android.core.Core
import com.mydomain.android.core.TimerPlan
import com.mydomain.android.core.Timers
import com.mydomain.android.core.friendlyError
import com.mydomain.android.core.niceName
import com.mydomain.android.ffi.TimerItem
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import java.text.DateFormat
import java.util.Date

private const val MINUTE = 60_000L

/**
 * Timers (pushed from Devices): countdown timers shared with your other
 * devices ([Timers]). Start one (a quick length or your own, a label, and
 * which devices ring when it ends); every device of yours with timers
 * shows it counting down, and Pause, Resume, +1 min, Cancel and Stop work
 * from any of them. The notification ([TimerAlarms]) shows the time left
 * and rings here.
 */
@Composable
fun TimersScreen(onBack: () -> Unit, onHelp: () -> Unit) {
    var now by remember { mutableLongStateOf(System.currentTimeMillis()) }
    LaunchedEffect(Unit) {
        while (true) {
            delay(250)
            Core.awaitOnScreen()
            now = System.currentTimeMillis()
        }
    }
    // This phone first, then your other devices that have timers.
    var others by remember { mutableStateOf<List<String>>(emptyList()) }
    LaunchedEffect(Core.peers) { others = withContext(Dispatchers.IO) { Timers.devices() } }
    val me = Core.me?.nodeId.orEmpty()
    val devices = (listOf(me) + others.filter { it != me }).filter { it.isNotEmpty() }
    val list = Timers.sorted()
    val ctx = LocalContext.current

    LazyColumn(
        Modifier.fillMaxSize(),
        contentPadding = PaddingValues(start = 16.dp, end = 16.dp, bottom = 16.dp),
        verticalArrangement = Arrangement.spacedBy(10.dp),
    ) {
        item {
            Row(Modifier.padding(top = 4.dp), verticalAlignment = Alignment.CenterVertically) {
                IconButton(onClick = onBack) {
                    Icon(Icons.AutoMirrored.Filled.ArrowBack, stringResource(R.string.back), tint = T.text)
                }
                Text(
                    stringResource(R.string.timers_title), Modifier.weight(1f).semantics { heading() },
                    style = MaterialTheme.typography.headlineSmall, color = T.text,
                )
                IconButton(onClick = onHelp) {
                    Icon(Icons.Outlined.Info, stringResource(R.string.help_open), tint = T.text2)
                }
            }
            Muted(stringResource(R.string.timers_subtitle))
        }
        // Rings late without exact alarms: say so, with the way to allow them.
        if (list.any { me in it.ringOn } && TimerAlarms.mayBeLate(ctx)) {
            item {
                Tile {
                    Muted(stringResource(R.string.timers_may_be_late))
                    GhostButton(stringResource(R.string.notes_remind_allow_exact), { NoteReminders.askExact(ctx) }, small = true)
                }
            }
        }
        item { NewTimer(devices, me) }
        if (list.isEmpty() && Timers.loaded) {
            item { EmptyState(stringResource(R.string.timers_empty_title), stringResource(R.string.timers_empty_body)) }
        }
        items(list, key = { it.id }) { TimerRow(it, now, devices, me) }
    }
}

/** A device's name here: "This phone" for this one. */
@Composable
private fun deviceName(node: String, me: String): String =
    if (node == me) stringResource(R.string.timers_this_phone) else Core.peerName(node)

/** Tick boxes for the devices that ring. */
@Composable
private fun RingPicker(devices: List<String>, me: String, value: List<String>, onChange: (List<String>) -> Unit) {
    Column {
        Text(stringResource(R.string.timers_ring_on), style = MaterialTheme.typography.titleSmall, color = T.text)
        for (node in devices) {
            val on = node in value
            Row(
                Modifier.fillMaxWidth().heightIn(min = 48.dp).toggleable(
                    value = on, role = Role.Checkbox,
                    onValueChange = { v -> onChange(if (v) value.filter { it != node } + node else value.filter { it != node }) },
                ),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                Checkbox(
                    checked = on, onCheckedChange = null,
                    colors = CheckboxDefaults.colors(checkedColor = T.accent, uncheckedColor = T.controlBorder),
                )
                Text(deviceName(node, me), style = MaterialTheme.typography.bodyMedium, color = T.text)
            }
        }
    }
}

@Composable
private fun NumberBox(value: String, onChange: (String) -> Unit, label: String, modifier: Modifier) {
    OutlinedTextField(
        value, { v -> onChange(v.filter { it.isDigit() }.take(3)) }, modifier = modifier,
        label = { Text(label) }, singleLine = true,
        keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Number),
        shape = MaterialTheme.shapes.small,
        colors = OutlinedTextFieldDefaults.colors(
            focusedBorderColor = T.accent, unfocusedBorderColor = T.controlBorder,
            focusedLabelColor = T.accentFg, unfocusedLabelColor = T.text3, cursorColor = T.accent,
            focusedContainerColor = T.surface2, unfocusedContainerColor = T.surface2,
            focusedTextColor = T.text, unfocusedTextColor = T.text,
        ),
    )
}

@OptIn(ExperimentalLayoutApi::class)
@Composable
private fun NewTimer(devices: List<String>, me: String) {
    val toast = LocalToast.current
    val scope = rememberCoroutineScope()
    var label by rememberSaveable { mutableStateOf("") }
    var h by rememberSaveable { mutableStateOf("") }
    var m by rememberSaveable { mutableStateOf("5") }
    var s by rememberSaveable { mutableStateOf("") }
    // Rings here unless you choose otherwise.
    var ring by remember { mutableStateOf<List<String>?>(null) }
    val ringOn = ring ?: listOfNotNull(me.ifEmpty { null })
    var busy by remember { mutableStateOf(false) }
    val badLength = stringResource(R.string.timers_bad_length)
    val nowhere = stringResource(R.string.timers_rings_nowhere)

    fun start(ms: Long?) {
        if (ms == null) return toast(badLength)
        busy = true
        scope.launch {
            try {
                Timers.start(label, ms, ringOn)
                label = ""
                if (ringOn.isEmpty()) toast(nowhere)
            } catch (e: Exception) {
                toast(friendlyError(e))
            } finally {
                busy = false
            }
        }
    }

    Tile(title = stringResource(R.string.timers_new)) {
        Field(label, { label = it.take(60) }, stringResource(R.string.timers_label))
        FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalArrangement = Arrangement.spacedBy(4.dp)) {
            for (mins in TimerPlan.PRESETS) {
                val text = if (mins >= 60) stringResource(R.string.timers_hours_short, mins / 60)
                else stringResource(R.string.timers_mins_short, mins)
                SoftButton(text, { start(mins * MINUTE) }, enabled = !busy)
            }
        }
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.CenterVertically) {
            NumberBox(h, { h = it }, stringResource(R.string.timers_hours), Modifier.weight(1f))
            NumberBox(m, { m = it }, stringResource(R.string.timers_minutes), Modifier.weight(1f))
            NumberBox(s, { s = it }, stringResource(R.string.timers_seconds), Modifier.weight(1f))
        }
        val custom = TimerPlan.durationOf(h, m, s)
        PrimaryButton(stringResource(R.string.timers_start), { start(custom) }, Modifier.fillMaxWidth(), enabled = !busy && custom != null)
        RingPicker(devices, me, ringOn) { ring = it }
    }
}

@OptIn(ExperimentalLayoutApi::class)
@Composable
private fun TimerRow(x: TimerItem, now: Long, devices: List<String>, me: String) {
    val toast = LocalToast.current
    val scope = rememberCoroutineScope()
    var editRing by remember { mutableStateOf(false) }
    val ended = Timers.ended(x, now)
    val paused = x.state == "paused"
    val left = TimerPlan.clock(Timers.remaining(x, now))
    fun act(block: suspend () -> Unit) {
        scope.launch { try { block() } catch (e: Exception) { toast(friendlyError(e)) } }
    }
    val status = when {
        ended -> stringResource(R.string.timers_ended)
        paused -> stringResource(R.string.timers_paused)
        else -> stringResource(R.string.timers_ends_at, DateFormat.getTimeInstance(DateFormat.SHORT).format(Date(x.endsAt)))
    }
    val ringNames = if (x.ringOn.isEmpty()) stringResource(R.string.timers_nowhere)
    else x.ringOn.map { deviceName(it, me) }.joinToString(", ")
    val leftLabel = stringResource(R.string.timers_left, left)

    Tile {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Text(
                x.label.ifBlank { stringResource(R.string.timer_untitled) }, Modifier.weight(1f),
                style = MaterialTheme.typography.titleMedium, color = T.text, maxLines = 1,
            )
            Muted(status)
        }
        Text(
            if (ended) "0:00" else left,
            Modifier.semantics { contentDescription = if (ended) status else leftLabel },
            fontSize = 40.sp, fontWeight = FontWeight.Bold,
            color = when {
                ended -> T.warning
                paused -> T.text2
                else -> T.text
            },
        )
        LinearProgressIndicator(
            progress = { Timers.progress(x, now) },
            modifier = Modifier.fillMaxWidth(),
            color = if (paused) T.text3 else T.accent,
            trackColor = T.surface2,
        )
        Row(verticalAlignment = Alignment.CenterVertically) {
            Muted(stringResource(R.string.timers_rings_on, ringNames), Modifier.weight(1f))
            GhostButton(stringResource(R.string.timers_change), { editRing = !editRing }, small = true)
        }
        if (editRing) {
            // Devices it rings on that aren't listed (not linked now) stay.
            val choices = devices + x.ringOn.filter { it !in devices }
            RingPicker(choices, me, x.ringOn) { v -> act { Timers.setRing(x.id, v) } }
        }
        if (x.byName.isNotBlank()) Muted(stringResource(R.string.timers_by, niceName(x.byName)))
        FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalArrangement = Arrangement.spacedBy(4.dp)) {
            when {
                ended -> {
                    PrimaryButton(stringResource(R.string.timer_stop), { act { Timers.action(x.id, "dismiss") } })
                    SoftButton(stringResource(R.string.timers_restart), { act { Timers.action(x.id, "restart") } })
                }
                paused -> PrimaryButton(stringResource(R.string.timer_resume), { act { Timers.action(x.id, "resume") } })
                else -> SoftButton(stringResource(R.string.timer_pause), { act { Timers.action(x.id, "pause") } })
            }
            SoftButton(stringResource(R.string.timer_plus_minute), { act { Timers.add(x.id, MINUTE) } })
            if (!ended) GhostButton(stringResource(R.string.timer_cancel), { act { Timers.action(x.id, "cancel") } })
        }
    }
}

/** The Timers entry on Devices: how many run, and the soonest. */
@Composable
fun TimersEntry(onOpen: () -> Unit) {
    var now by remember { mutableLongStateOf(System.currentTimeMillis()) }
    LaunchedEffect(Unit) {
        while (true) {
            delay(1000)
            Core.awaitOnScreen()
            now = System.currentTimeMillis()
        }
    }
    val list = Timers.sorted()
    val running = list.count { it.state == "running" && !Timers.ended(it, now) }
    val ringing = list.count { Timers.ended(it, now) }
    val next = list.firstOrNull { it.state == "running" && !Timers.ended(it, now) }
    Tile(onClick = onOpen) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Column(Modifier.weight(1f)) {
                Text(stringResource(R.string.timers_title), style = MaterialTheme.typography.titleMedium, color = T.text)
                Muted(
                    when {
                        ringing > 0 -> stringResource(R.string.timer_times_up)
                        running > 0 -> pluralStringResource(R.plurals.timers_running, running, running)
                        list.isNotEmpty() -> stringResource(R.string.timers_paused)
                        else -> stringResource(R.string.timers_entry_hint)
                    },
                )
            }
            if (next != null) {
                Text(
                    TimerPlan.clock(Timers.remaining(next, now)),
                    fontSize = 22.sp, fontWeight = FontWeight.Bold, color = T.text,
                )
            }
        }
    }
}
