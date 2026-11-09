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

internal val BOTH = setOf("phone", "computer")
internal val COMPUTER = setOf("computer")

/** One tile a device's screen can show: [id] is kept in saved layouts
 *  (never rename one); [available]: the device offers it now, from its
 *  status (else it's left out, and listed as such in Arrange). [content]
 *  gets the status too, so the list itself never changes with it. */
internal class PageTile(
    val id: String,
    val title: Int,
    val kinds: Set<String> = BOTH,
    val available: (DeviceStatus?) -> Boolean = { true },
    val content: @Composable (DeviceStatus?) -> Unit,
)

/** Every tile a device's screen can show, in default order. A new tile is
 *  one entry here; a saved layout that predates it shows it at this place.
 *  Made once per device (and what a search opened), not per status
 *  update: what changes is passed to [PageTile.available] and
 *  [PageTile.content]. */
internal fun deviceTiles(node: String, name: String, foundMessage: Long?, foundClip: Long?): List<PageTile> = listOf(
    PageTile("chat", R.string.chat_title) { ChatTile(node, foundMessage) },
    PageTile("files", R.string.files_title) { FilesTile(node) },
    // Its shared folders, for a device that can be browsed.
    PageTile("browse", R.string.browse_title, available = { it?.sharedFolders != null }) { BrowseTile(node, name) },
    // Clipboard: the last part of Files.
    PageTile("clipboard", R.string.clipboard_title) { ClipboardTile(node) },
    // Text sent to it and from it as clipboard, kept on this phone.
    PageTile("clipboard-history", R.string.clip_history_title, available = { ClipLog.enabled }) {
        ClipHistoryTile(node, name, foundClip)
    },
    // Find & control, in parts (the "control" id stays: saved layouts).
    PageTile("control", R.string.ctl_group_find) { FindTile(node, name) },
    PageTile("control-sound", R.string.ctl_group_sound, available = { st -> st != null && st.actions.any { it in SOUND } }) { st ->
        st?.let { SoundTile(node, it) }
    },
    PageTile("control-screen", R.string.ctl_group_screen, available = { st -> st != null && st.actions.any { it in SCREEN } }) { st ->
        st?.let { ScreenPowerTile(node, it) }
    },
    PageTile("control-open", R.string.ctl_group_open) { OpenOnTile(node, name) },
    // What it said about itself: battery, storage, memory, processor, uptime.
    PageTile("info", R.string.dev_card_info, available = { st -> st != null && hasInfo(st) }) { st ->
        st?.let { InfoTile(node, name, it) }
    },
    // How this phone reaches the computer, and how fast.
    PageTile("connection", R.string.conn_title, COMPUTER) { ConnectionTile(node, name) },
    // Commands its owner lets your devices run (it lists "run").
    PageTile("commands", R.string.run_title, available = { it?.actions?.contains("run") == true }) { CommandsTile(node, name) },
    // A computer that lets the phone control it (it lists "input").
    PageTile("touchpad", R.string.touchpad_title, COMPUTER, available = { it?.actions?.contains("input") == true }) { st ->
        st?.let { TouchpadTile(node, it) }
    },
    // A computer that can show this phone's screen (it says screen_view).
    PageTile("screen-share", R.string.screen_share_title, COMPUTER, available = { it?.screenView == true }) {
        ShareScreenTile(node, name)
    },
    // What plays there, with play/pause, skip, seek and volume when its
    // app takes media commands (media_ctl); an older one just shows it.
    PageTile(
        "now-playing", R.string.media_title,
        available = { it?.mediaCtl == true || Core.nowPlaying[node] != null },
    ) { MediaTile(node, name) },
    // A computer that plays this phone's audio (it says audio_sink),
    // from a phone that can capture it (Android 10 or later).
    // (Phones say audio_sink too now, for a computer's sound: only
    // a computer's page offers this.)
    PageTile(
        "phone-audio", R.string.audio_tile_title, COMPUTER,
        available = { st -> st != null && st.audioSink && st.platform != "android" && PhoneAudio.supported },
    ) { PhoneAudioTile(node, name) },
    // A computer that plays its sound here (it says audio_source).
    PageTile(
        "computer-audio", R.string.caudio_tile_title, COMPUTER,
        available = { st -> st != null && st.audioSource && st.platform != "android" },
    ) { ComputerAudioTile(node, name) },
    // A computer that can show its own notifications here (it says notif_share).
    PageTile("computer-notifs", R.string.computer_notifs_title, COMPUTER, available = { it?.notifShare != null }) { st ->
        st?.notifShare?.let { ComputerNotifsTile(node, name, it) }
    },
    PageTile("notify", R.string.notify_title) { NotifyTile() },
    PageTile("apps", R.string.apps_title) { AppsTile(node) },
)
