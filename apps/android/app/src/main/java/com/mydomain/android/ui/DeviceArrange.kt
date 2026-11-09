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

/** A long press on a tile (not on a button in it) starts Arrange; TalkBack
 *  offers it as the tile's long-press action. */
internal fun Modifier.longPressToArrange(node: String, label: String, onArrange: () -> Unit): Modifier = this
    .pointerInput(node) { detectTapGestures(onLongPress = { onArrange() }) }
    .semantics { onLongClick(label) { onArrange(); true } }

/** Arrange: each shown tile as a card with Move up / Move down / Hide, the
 *  hidden ones listed after to show again, "Only for this device" and
 *  Reset. Saved as it changes. */
internal fun LazyListScope.arrangeItems(
    node: String,
    name: String,
    kind: String,
    tiles: List<PageTile>,
    st: DeviceStatus?,
    layout: PageLayoutView,
    said: String,
    say: (String) -> Unit,
    onDone: () -> Unit,
) {
    val onlyThis = layout.scope == LayoutScope.DEVICE
    val byId = tiles.associateBy { it.id }
    val shown = layout.shown
    item(key = "arrange-bar") {
        val ctx = LocalContext.current
        Tile(title = stringResource(R.string.layout_title)) {
            Muted(stringResource(R.string.layout_hint))
            SwitchRow(stringResource(R.string.layout_only_this), onlyThis) { on ->
                // On: this device keeps what it shows now as its own. Off: it
                // follows its kind's layout again.
                PageLayoutStore.set(ctx, kind, node, true, if (on) PageLayout(layout.order, layout.hidden) else null)
            }
            Muted(
                when {
                    onlyThis -> stringResource(R.string.layout_only_this_on, name)
                    kind == "phone" -> stringResource(R.string.layout_all_phones)
                    else -> stringResource(R.string.layout_all_computers)
                },
            )
            val resetDone = stringResource(R.string.layout_reset_done)
            Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                if (layout.scope != LayoutScope.DEFAULT) {
                    GhostButton(stringResource(R.string.layout_reset), {
                        PageLayoutStore.set(ctx, kind, node, onlyThis, null)
                        say(resetDone)
                    })
                }
                Spacer(Modifier.weight(1f))
                PrimaryButton(stringResource(R.string.layout_done), onDone)
            }
            if (said.isNotEmpty()) Muted(said, Modifier.announce())
        }
    }
    for (id in shown) item(key = "arrange:$id") {
        val t = byId[id] ?: return@item
        val ctx = LocalContext.current
        val title = stringResource(t.title)
        val at = shown.indexOf(id)
        val upLabel = stringResource(R.string.layout_up_label, title)
        val downLabel = stringResource(R.string.layout_down_label, title)
        val hideLabel = stringResource(R.string.layout_hide_label, title)
        val hiddenNow = stringResource(R.string.layout_hidden_now, title)
        val res = ctx.resources
        fun step(s: Int) {
            val order = PageArrangement.moveStep(layout.order, layout.hidden, id, s)
            if (order === layout.order) return
            PageLayoutStore.set(ctx, kind, node, onlyThis, PageLayout(order, layout.hidden))
            val now = order.filter { it !in layout.hidden }
            say(res.getString(R.string.layout_moved_to, title, now.indexOf(id) + 1, now.size))
        }
        Tile(title = title) {
            if (!t.available(st)) Muted(stringResource(R.string.layout_not_now, name))
            Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                SoftButton(
                    stringResource(R.string.layout_up), { step(-1) },
                    Modifier.semantics { contentDescription = upLabel }, enabled = at > 0,
                )
                SoftButton(
                    stringResource(R.string.layout_down), { step(1) },
                    Modifier.semantics { contentDescription = downLabel }, enabled = at < shown.size - 1,
                )
                Spacer(Modifier.weight(1f))
                GhostButton(stringResource(R.string.layout_hide), {
                    PageLayoutStore.set(ctx, kind, node, onlyThis, PageLayout(layout.order, layout.hidden + id))
                    say(hiddenNow)
                }, Modifier.semantics { contentDescription = hideLabel }, small = true)
            }
        }
    }
    val hidden = layout.order.filter { it in layout.hidden }
    if (hidden.isNotEmpty()) item(key = "arrange-hidden") {
        val ctx = LocalContext.current
        Tile(title = stringResource(R.string.layout_hidden)) {
            for (id in hidden) {
                val t = byId[id] ?: continue
                val title = stringResource(t.title)
                val showLabel = stringResource(R.string.layout_show_label, title)
                val shownNow = stringResource(R.string.layout_shown_now, title)
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Text(title, Modifier.weight(1f), style = MaterialTheme.typography.bodyMedium, color = T.text)
                    GhostButton(stringResource(R.string.layout_show), {
                        PageLayoutStore.set(ctx, kind, node, onlyThis, PageLayout(layout.order, layout.hidden - id))
                        say(shownNow)
                    }, Modifier.semantics { contentDescription = showLabel }, small = true)
                }
            }
        }
    }
}
