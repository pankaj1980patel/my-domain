package com.mydomain.android.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.gestures.awaitEachGesture
import androidx.compose.foundation.gestures.awaitFirstDown
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.input.pointer.positionChange
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import com.mydomain.android.R
import com.mydomain.android.core.Core
import com.mydomain.android.core.DeviceStatus
import com.mydomain.android.core.friendlyError
import kotlinx.coroutines.launch
import kotlin.math.abs

/** Pointer pixels on the computer per finger pixel on the pad. */
private const val SPEED = 1.6f
/** A touch shorter than this that barely moved is a tap (a click). */
private const val TAP_MS = 250L
private const val TAP_SLOP_PX = 24f
/** How far three or four fingers must travel (on average) to be a swipe. */
private const val SWIPE_PX = 120f

/**
 * The phone as a touchpad and keyboard for a computer that lists "input"
 * in its status (see mdcore `features::input`): drag one finger to move the
 * pointer, tap to click (two fingers: right click, three: middle click),
 * drag two fingers to scroll, swipe three or four fingers for the
 * computer's own shortcuts (switch apps or desktops, show all windows, show
 * the desktop), and type into the field. Enlarge opens the pad full screen.
 * Moves go to the core on every touch event; the core batches them.
 */
@Composable
fun TouchpadTile(node: String, status: DeviceStatus) {
    val toast = LocalToast.current
    val scope = rememberCoroutineScope()
    var text by remember { mutableStateOf("") }
    var enlarged by remember { mutableStateOf(false) }
    val click = { button: String ->
        scope.launch { try { Core.inputClick(node, button) } catch (e: Exception) { toast(friendlyError(e)) } }
        Unit
    }
    val key = { k: String ->
        scope.launch { try { Core.inputKey(node, k) } catch (e: Exception) { toast(friendlyError(e)) } }
        Unit
    }

    Tile(title = stringResource(R.string.touchpad_title)) {
        Muted(
            when (status.inputNeeds) {
                "accessibility" -> stringResource(R.string.touchpad_needs_accessibility)
                "display" -> stringResource(R.string.touchpad_needs_display)
                else -> stringResource(R.string.touchpad_body)
            }
        )
        Pad(node, click, Modifier.fillMaxWidth().height(220.dp))
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            GhostButton(stringResource(R.string.touchpad_left_click), { click("left") }, Modifier.weight(1f), small = true)
            GhostButton(stringResource(R.string.touchpad_right_click), { click("right") }, Modifier.weight(1f), small = true)
            GhostButton(stringResource(R.string.touchpad_enlarge), { enlarged = true }, Modifier.weight(1f), small = true)
        }
        Row(verticalAlignment = Alignment.CenterVertically) {
            Field(text, { text = it }, stringResource(R.string.touchpad_type_hint), Modifier.weight(1f))
            Spacer(Modifier.width(8.dp))
            PrimaryButton(stringResource(R.string.touchpad_type), {
                val t = text
                scope.launch {
                    try { Core.inputText(node, t); text = "" } catch (e: Exception) { toast(friendlyError(e)) }
                }
            }, enabled = text.isNotEmpty())
        }
        Row(horizontalArrangement = Arrangement.spacedBy(6.dp)) {
            listOf(
                "left" to R.string.touchpad_key_left, "up" to R.string.touchpad_key_up,
                "down" to R.string.touchpad_key_down, "right" to R.string.touchpad_key_right,
            ).forEach { (k, label) -> GhostButton(stringResource(label), { key(k) }, Modifier.weight(1f), small = true) }
        }
        Row(horizontalArrangement = Arrangement.spacedBy(6.dp)) {
            listOf(
                "enter" to R.string.touchpad_key_enter, "backspace" to R.string.touchpad_key_backspace,
                "esc" to R.string.touchpad_key_esc,
            ).forEach { (k, label) -> GhostButton(stringResource(label), { key(k) }, Modifier.weight(1f), small = true) }
        }
    }
    if (enlarged) {
        Dialog(
            onDismissRequest = { enlarged = false },
            properties = DialogProperties(usePlatformDefaultWidth = false),
        ) {
            Column(
                Modifier.fillMaxSize().background(T.bg).padding(12.dp),
                verticalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                Pad(node, click, Modifier.fillMaxWidth().weight(1f))
                Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    GhostButton(stringResource(R.string.touchpad_left_click), { click("left") }, Modifier.weight(1f))
                    GhostButton(stringResource(R.string.touchpad_right_click), { click("right") }, Modifier.weight(1f))
                    PrimaryButton(stringResource(R.string.touchpad_shrink), { enlarged = false }, Modifier.weight(1f))
                }
            }
        }
    }
}

/**
 * The pad itself. One finger moves, two scroll; a quick touch that barely
 * moved clicks (left, right or middle by fingers). Once three or four
 * fingers are down, nothing moves: where they went decides the swipe, sent
 * when they lift.
 */
@Composable
private fun Pad(node: String, click: (String) -> Unit, modifier: Modifier) {
    val toast = LocalToast.current
    val scope = rememberCoroutineScope()
    Box(
        modifier
            .background(T.surface2, MaterialTheme.shapes.small)
            .border(1.dp, T.border, MaterialTheme.shapes.small)
            .pointerInput(node) {
                awaitEachGesture {
                    awaitFirstDown(requireUnconsumed = false)
                    val start = System.currentTimeMillis()
                    var travelled = 0f
                    var fingers = 1
                    // Where three or four fingers went, on average.
                    var swipeX = 0f
                    var swipeY = 0f
                    // Fractions of a pixel left over, sent with the next move.
                    var restX = 0f
                    var restY = 0f
                    var warned = false
                    while (true) {
                        val event = awaitPointerEvent()
                        val down = event.changes.filter { it.pressed }
                        if (down.isEmpty()) break
                        fingers = maxOf(fingers, down.size)
                        var dx = 0f
                        var dy = 0f
                        down.forEach { val d = it.positionChange(); dx += d.x; dy += d.y }
                        dx /= down.size
                        dy /= down.size
                        travelled += abs(dx) + abs(dy)
                        event.changes.forEach { it.consume() }
                        if (fingers >= 3) {
                            swipeX += dx
                            swipeY += dy
                            continue
                        }
                        val two = down.size >= 2
                        // Two fingers up scrolls down, as on a laptop's touchpad.
                        restX += if (two) -dx else dx * SPEED
                        restY += if (two) -dy else dy * SPEED
                        val ix = restX.toInt()
                        val iy = restY.toInt()
                        if (ix == 0 && iy == 0) continue
                        restX -= ix
                        restY -= iy
                        val err = if (two) Core.inputScroll(node, ix, iy) else Core.inputMove(node, ix, iy)
                        if (err != null && !warned) {
                            warned = true
                            toast(friendlyError(err))
                        }
                    }
                    val quick = System.currentTimeMillis() - start < TAP_MS
                    val swipe = swipeDir(swipeX, swipeY)
                    when {
                        fingers >= 3 && swipe != null -> scope.launch {
                            try { Core.inputGesture(node, minOf(fingers, 4), swipe) } catch (e: Exception) { toast(friendlyError(e)) }
                        }
                        quick && travelled < TAP_SLOP_PX -> click(
                            when (fingers) { 1 -> "left"; 2 -> "right"; else -> "middle" }
                        )
                    }
                }
            },
        contentAlignment = Alignment.Center,
    ) { Muted(stringResource(R.string.touchpad_pad_hint), center = true) }
}

/** "left", "right", "up" or "down" for a swipe that went far enough. */
private fun swipeDir(dx: Float, dy: Float): String? = when {
    maxOf(abs(dx), abs(dy)) < SWIPE_PX -> null
    abs(dx) > abs(dy) -> if (dx < 0) "left" else "right"
    else -> if (dy < 0) "up" else "down"
}
