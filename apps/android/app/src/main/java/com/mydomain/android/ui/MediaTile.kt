package com.mydomain.android.ui

import android.graphics.BitmapFactory
import android.util.Base64
import androidx.compose.foundation.Image
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Slider
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableLongStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import com.mydomain.android.R
import com.mydomain.android.core.Core
import com.mydomain.android.core.MediaPlan
import com.mydomain.android.core.friendlyError
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch

/**
 * What plays on another of your devices ([node], [name]), with previous /
 * play-pause / next, a seek bar, its volume and which app to show when
 * several play (core `media_command`). While shown it asks the device to
 * keep it posted (`media_watch`, every few minutes); the position runs on
 * here between updates. A device with an older app shows what plays, no
 * controls.
 */
@Composable
fun MediaTile(node: String, name: String) {
    val toast = LocalToast.current
    val scope = rememberCoroutineScope()
    val controls = Core.deviceStatus[node]?.mediaCtl == true
    val p = Core.nowPlaying[node]
    var now by remember { mutableLongStateOf(System.currentTimeMillis()) }
    var dragging by remember { mutableStateOf<Float?>(null) }

    LaunchedEffect(node, controls) {
        if (!controls) return@LaunchedEffect
        while (true) {
            Core.awaitOnScreen()
            Core.mediaWatch(node)
            delay(MediaPlan.WATCH_EVERY_MS)
        }
    }
    LaunchedEffect(node) {
        while (true) {
            delay(1000)
            Core.awaitOnScreen()
            now = System.currentTimeMillis()
        }
    }

    fun cmd(action: String, value: Long? = null, session: String? = p?.session) {
        scope.launch {
            try {
                Core.mediaCommand(node, action, value, session)
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                toast(friendlyError(e))
            }
        }
    }

    Tile(title = stringResource(R.string.media_title)) {
        if (p == null) {
            Muted(stringResource(R.string.media_nothing, name))
            return@Tile
        }
        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
            val art = remember(p.art) {
                p.art?.let { b64 ->
                    runCatching {
                        val bytes = Base64.decode(b64, Base64.DEFAULT)
                        BitmapFactory.decodeByteArray(bytes, 0, bytes.size)?.asImageBitmap()
                    }.getOrNull()
                }
            }
            if (art != null) {
                Image(
                    art, stringResource(R.string.media_art, p.title),
                    Modifier.size(64.dp).clip(RoundedCornerShape(12.dp)), contentScale = ContentScale.Crop,
                )
            }
            Column(Modifier.weight(1f)) {
                Text(p.title, fontWeight = FontWeight.Bold, maxLines = 1, overflow = TextOverflow.Ellipsis)
                p.artist?.let { Text(it, style = MaterialTheme.typography.bodySmall, maxLines = 1, overflow = TextOverflow.Ellipsis) }
                Muted(
                    stringResource(
                        if (p.playing) R.string.media_playing_in else R.string.media_paused_in,
                        p.app ?: name,
                    ),
                )
            }
        }
        if (!controls) {
            Muted(stringResource(R.string.media_older_app, name))
            return@Tile
        }
        val duration = p.duration
        if (duration != null) {
            val pos = dragging?.let { MediaPlan.seekTarget(it, duration) } ?: p.positionNow(now) ?: 0L
            val label = stringResource(R.string.media_seek)
            Slider(
                value = (pos.toFloat() / duration).coerceIn(0f, 1f),
                onValueChange = { dragging = it },
                onValueChangeFinished = {
                    dragging?.let { cmd("seek", MediaPlan.seekTarget(it, duration)) }
                    dragging = null
                },
                enabled = p.canSeek,
                modifier = Modifier.fillMaxWidth().semantics {
                    contentDescription = label
                    stateDescription = MediaPlan.clock(pos)
                },
            )
            Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.SpaceBetween) {
                Muted(MediaPlan.clock(pos))
                Muted(MediaPlan.clock(duration))
            }
        }
        Row(
            Modifier.fillMaxWidth(),
            horizontalArrangement = Arrangement.spacedBy(8.dp, Alignment.CenterHorizontally),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            val prev = stringResource(R.string.media_previous)
            val playPause = stringResource(if (p.playing) R.string.media_pause else R.string.media_play)
            val next = stringResource(R.string.media_next)
            SoftButton("⏮", { cmd("previous") }, Modifier.semantics { contentDescription = prev }, enabled = p.canPrev)
            PrimaryButton(
                if (p.playing) "⏸" else "▶", { cmd(if (p.playing) "pause" else "play") },
                Modifier.semantics { contentDescription = playPause },
            )
            SoftButton("⏭", { cmd("next") }, Modifier.semantics { contentDescription = next }, enabled = p.canNext)
        }
        Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            val down = stringResource(R.string.media_volume_down)
            val up = stringResource(R.string.media_volume_up)
            GhostButton("−", { cmd("volume_down", session = null) }, Modifier.semantics { contentDescription = down }, small = true)
            val max = p.volumeMax
            if (max != null) {
                val label = stringResource(R.string.media_volume)
                var level by remember(p.volume) { mutableStateOf((p.volume ?: 0).toFloat()) }
                Slider(
                    value = level,
                    onValueChange = { level = it },
                    onValueChangeFinished = { cmd("volume_set", level.toLong(), session = null) },
                    valueRange = 0f..max.toFloat(),
                    steps = (max - 1).coerceIn(0, 100),
                    modifier = Modifier.weight(1f).semantics { contentDescription = label },
                )
            } else {
                Muted(stringResource(R.string.media_volume), Modifier.weight(1f), center = true)
            }
            GhostButton("+", { cmd("volume_up", session = null) }, Modifier.semantics { contentDescription = up }, small = true)
        }
        if (p.sessions.size > 1) {
            Muted(stringResource(R.string.media_session))
            Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                p.sessions.forEach { (id, label) ->
                    SoftButton(label, { cmd("session", session = id) }, Modifier.fillMaxWidth(), selected = id == p.session)
                }
            }
        }
    }
}
