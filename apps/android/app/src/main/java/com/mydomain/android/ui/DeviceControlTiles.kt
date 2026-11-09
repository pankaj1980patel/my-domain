package com.mydomain.android.ui

import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.heightIn
import androidx.compose.material3.IconButtonDefaults
import androidx.compose.material3.FilledIconButton
import androidx.compose.material3.TextButton
import java.util.Locale
import androidx.compose.foundation.layout.height
import androidx.compose.runtime.derivedStateOf
import android.content.Context
import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.activity.compose.BackHandler
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.DropdownMenu
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.ui.semantics.selected
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.onLongClick
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.lazy.LazyListScope
import com.mydomain.android.core.DeviceIconShapes
import com.mydomain.android.core.DeviceKinds
import com.mydomain.android.core.DeviceLabels
import com.mydomain.android.core.LayoutScope
import com.mydomain.android.core.PageArrangement
import com.mydomain.android.core.PageLayout
import com.mydomain.android.core.PageLayoutStore
import com.mydomain.android.core.PageLayoutView
import androidx.compose.ui.platform.LocalContext
import android.net.Uri
import android.content.Intent
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import org.json.JSONObject
import com.mydomain.android.core.DeviceStatus
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
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import com.mydomain.android.ComputerNotifs
import com.mydomain.android.PhoneAudio
import com.mydomain.android.R
import com.mydomain.android.core.ChatHistory
import com.mydomain.android.core.ChatMessage
import com.mydomain.android.core.ChatPage
import com.mydomain.android.core.ChatStatus
import com.mydomain.android.core.ClipLog
import com.mydomain.android.core.Core
import com.mydomain.android.core.LinkState
import com.mydomain.android.core.friendlyError
import com.mydomain.android.core.linkStateOf
import com.mydomain.android.core.progressLabel
import kotlinx.coroutines.launch

// The device page's Control tiles (DeviceTiles.kt lists them).

/** Where another device is (find my computer): Locate, the position, and
 *  a map. Greyed out, saying why, until that device allows it. */
@Composable
private fun LocateRow(node: String, name: String, status: DeviceStatus) {
    val toast = LocalToast.current
    val scope = rememberCoroutineScope()
    val ctx = LocalContext.current
    val asking = stringResource(R.string.locate_asking, name)
    val noMap = stringResource(R.string.setup_no_screen)
    val place = status.location
    if (place != null) {
        val mins = ((System.currentTimeMillis() / 1000 - place.at) / 60).coerceAtLeast(0)
        // Coordinates in plain digits with a point, whatever the language.
        Text(
            stringResource(
                R.string.locate_where,
                String.format(Locale.ROOT, "%.5f", place.lat), String.format(Locale.ROOT, "%.5f", place.lon), place.acc.toInt(), mins,
            ),
            Modifier.announce(), style = MaterialTheme.typography.bodyMedium, color = T.text,
        )
    }
    Row(verticalAlignment = Alignment.CenterVertically) {
        SoftButton(stringResource(R.string.locate), {
            scope.launch {
                try {
                    Core.runAction(node, "locate")
                    toast(asking)
                } catch (e: Exception) { toast(friendlyError(e)) }
            }
        }, Modifier.weight(1f), enabled = status.locationSharing)
        if (place != null) {
            Spacer(Modifier.width(8.dp))
            GhostButton(stringResource(R.string.locate_map), {
                val geo = Intent(Intent.ACTION_VIEW, Uri.parse("geo:${place.lat},${place.lon}?q=${place.lat},${place.lon}"))
                val web = Intent(Intent.ACTION_VIEW, Uri.parse("https://www.openstreetmap.org/?mlat=${place.lat}&mlon=${place.lon}#map=17/${place.lat}/${place.lon}"))
                if (!tryOpen(ctx, geo, web)) toast(noMap)
            })
        }
    }
    if (!status.locationSharing) Muted(stringResource(R.string.locate_off, name))
}

/** Actions of the "Sound & media" part, and of "Screen & power". */
internal val SOUND = setOf("volume", "mute", "ringer", "media")
internal val SCREEN = setOf("flashlight", "keep_awake", "lock", "slides")

/** Run one of the device's own actions, saying why if it fails. */
@Composable
private fun rememberRun(node: String): (String, JSONObject) -> Unit {
    val toast = LocalToast.current
    val scope = rememberCoroutineScope()
    return { name, args ->
        scope.launch {
            try { Core.runAction(node, name, args) } catch (e: Exception) { toast(friendlyError(e)) }
        }
    }
}

/** A row of equal buttons for [node]'s action, under its own label line. */
@Composable
private fun Btn(label: Int, run: () -> Unit, m: Modifier = Modifier) =
    GhostButton(stringResource(label), run, m, small = true)

/** Find: ring it, and where it is (when it lets your devices ask). Asks
 *  for its status when the page opens over a live link. */
@Composable
internal fun FindTile(node: String, name: String) {
    val toast = LocalToast.current
    val scope = rememberCoroutineScope()
    val status = Core.deviceStatus[node]
    val ringing = stringResource(R.string.find_ringing, name)
    val ringQueued = stringResource(R.string.find_ring_queued, name)

    // Ask for its status when the screen opens; the reply fills `status`
    // (and the info tile). Asked only over a live link (and again when one
    // comes up): asking a device that isn't connected would wake it by
    // push just to show this.
    val linked = Core.links.containsKey(node)
    LaunchedEffect(node, linked) { if (linked) runCatching { Core.requestStatus(node) } }

    Tile(title = stringResource(R.string.ctl_group_find)) {
        Muted(stringResource(R.string.find_body))
        SoftButton(stringResource(R.string.find_ring), {
            scope.launch {
                try {
                    val via = Core.ring(node)
                    toast(if (via == "queued") ringQueued else ringing)
                } catch (e: Exception) { toast(friendlyError(e)) }
            }
        }, Modifier.fillMaxWidth())
        if (status != null && "locate" in status.actions) LocateRow(node, name, status)
    }
}

/** Sound & media: volume, mute, the ringer and the media keys, each only
 *  when the device listed it. */
@Composable
internal fun SoundTile(node: String, st: DeviceStatus) {
    val run = rememberRun(node)
    Tile(title = stringResource(R.string.ctl_group_sound)) {
        if ("volume" in st.actions || "mute" in st.actions) {
            Muted(st.volume?.let { stringResource(R.string.ctl_volume_level, it) } ?: stringResource(R.string.ctl_volume), Modifier.announce())
            Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                if ("volume" in st.actions) {
                    Btn(R.string.ctl_volume_down, { run("volume", JSONObject().put("step", -1)) }, Modifier.weight(1f))
                    Btn(R.string.ctl_volume_up, { run("volume", JSONObject().put("step", 1)) }, Modifier.weight(1f))
                }
                if ("mute" in st.actions) {
                    Btn(
                        if (st.muted == true) R.string.ctl_unmute else R.string.ctl_mute,
                        { run("mute", JSONObject().put("on", st.muted != true)) }, Modifier.weight(1f),
                    )
                }
            }
        }
        if ("ringer" in st.actions) {
            Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                listOf("normal" to R.string.ctl_ring_normal, "vibrate" to R.string.ctl_ring_vibrate, "silent" to R.string.ctl_ring_silent)
                    .forEach { (mode, label) ->
                        // The current mode is the filled one, and says "selected".
                        val m = Modifier.weight(1f)
                        if (st.ringer == mode) SoftButton(stringResource(label), { }, m, selected = true)
                        else Btn(label, { run("ringer", JSONObject().put("mode", mode)) }, m.semantics { selected = false })
                    }
            }
        }
        if ("media" in st.actions) {
            Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                Btn(R.string.ctl_previous, { run("media", JSONObject().put("key", "previous")) }, Modifier.weight(1f))
                Btn(R.string.ctl_play_pause, { run("media", JSONObject().put("key", "play_pause")) }, Modifier.weight(1f))
                Btn(R.string.ctl_next, { run("media", JSONObject().put("key", "next")) }, Modifier.weight(1f))
            }
        }
    }
}

/** Screen & power: the flashlight, keeping it awake, locking it and the
 *  slide remote, each only when the device listed it. */
@Composable
internal fun ScreenPowerTile(node: String, st: DeviceStatus) {
    val run = rememberRun(node)
    Tile(title = stringResource(R.string.ctl_group_screen)) {
        if ("flashlight" in st.actions) {
            val on = st.torch == true
            SoftButton(
                stringResource(if (on) R.string.ctl_torch_off else R.string.ctl_torch_on),
                { run("flashlight", JSONObject().put("on", !on)) }, Modifier.fillMaxWidth(),
            )
        }
        if ("keep_awake" in st.actions) {
            Muted(stringResource(R.string.ctl_awake))
            Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                Btn(R.string.ctl_awake_30, { run("keep_awake", JSONObject().put("minutes", 30)) }, Modifier.weight(1f))
                Btn(R.string.ctl_awake_60, { run("keep_awake", JSONObject().put("minutes", 60)) }, Modifier.weight(1f))
                Btn(R.string.ctl_awake_off, { run("keep_awake", JSONObject().put("minutes", 0)) }, Modifier.weight(1f))
            }
        }
        if ("lock" in st.actions) {
            Btn(R.string.ctl_lock, { run("lock", JSONObject()) }, Modifier.fillMaxWidth())
        }
        if ("slides" in st.actions) {
            Muted(
                when (st.slidesNeeds) {
                    "accessibility" -> stringResource(R.string.ctl_slides_needs_accessibility)
                    "xdotool", "wtype" -> stringResource(R.string.ctl_slides_needs_tool)
                    else -> stringResource(R.string.ctl_slides)
                },
            )
            Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                Btn(R.string.ctl_slide_previous, { run("slides", JSONObject().put("key", "previous")) }, Modifier.weight(1f))
                Btn(R.string.ctl_slide_black, { run("slides", JSONObject().put("key", "black")) }, Modifier.weight(1f))
                Btn(R.string.ctl_slide_next, { run("slides", JSONObject().put("key", "next")) }, Modifier.weight(1f))
            }
        }
    }
}

/** Open on it: a web link, and a search in an app when it can. */
@Composable
internal fun OpenOnTile(node: String, name: String) {
    val toast = LocalToast.current
    val scope = rememberCoroutineScope()
    val run = rememberRun(node)
    val st = Core.deviceStatus[node]
    var url by rememberSaveable(node) { mutableStateOf("") }
    var busy by remember { mutableStateOf(false) }
    val linkSent = stringResource(R.string.find_link_sent, name)
    val linkQueued = stringResource(R.string.find_link_queued, name)
    val linkInvalid = stringResource(R.string.find_link_invalid)
    Tile(title = stringResource(R.string.ctl_group_open)) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Field(url, { url = it }, stringResource(R.string.find_link_hint), Modifier.weight(1f))
            Spacer(Modifier.width(8.dp))
            SoftButton(stringResource(R.string.send), {
                val link = withScheme(url)
                if (!isWebLink(link)) {
                    toast(linkInvalid)
                } else {
                    busy = true
                    scope.launch {
                        try {
                            val via = Core.openUrl(node, link)
                            url = ""
                            toast(if (via == "queued") linkQueued else linkSent)
                        } catch (e: Exception) { toast(friendlyError(e)) } finally { busy = false }
                    }
                }
            }, enabled = !busy && url.isNotBlank())
        }
        if (st != null && "search" in st.actions) {
            var query by remember { mutableStateOf("") }
            Field(query, { query = it }, stringResource(R.string.ctl_search_hint), Modifier.fillMaxWidth())
            Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                listOf("youtube" to R.string.ctl_search_youtube, "spotify" to R.string.ctl_search_spotify, "web" to R.string.ctl_search_web)
                    .forEach { (app, label) ->
                        Btn(label, {
                            if (query.isNotBlank()) run("search", JSONObject().put("app", app).put("query", query.trim()))
                        }, Modifier.weight(1f))
                    }
            }
        }
    }
}

/** "example.com/x" -> "https://example.com/x"; a link with a scheme is kept. */
private fun withScheme(input: String): String {
    val t = input.trim()
    return if (t.contains("://")) t else "https://$t"
}

private fun isWebLink(url: String): Boolean {
    val lower = url.lowercase()
    return (lower.startsWith("https://") || lower.startsWith("http://")) && url.length > "https://".length
}
