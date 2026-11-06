package com.mydomain.android.core

import android.content.Context
import android.net.Uri
import android.os.Handler
import android.os.Looper
import android.util.Log
import androidx.annotation.PluralsRes
import androidx.annotation.StringRes
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateListOf
import androidx.compose.runtime.mutableStateMapOf
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import com.mydomain.android.BuildConfig
import com.mydomain.android.HomeWidget
import com.mydomain.android.R
import com.mydomain.android.ShareTargets
import com.mydomain.android.WifiDirect
import com.mydomain.android.ffi.DeviceIdentity
import com.mydomain.android.ffi.MdEngine
import com.mydomain.android.ffi.PeerInfo
import com.mydomain.android.ffi.Session
import com.mydomain.android.ffi.runningEngine
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import org.json.JSONArray
import org.json.JSONObject

/**
 * One chat bubble. Outgoing messages carry [msgId] (sent to the core) and stay
 * [pending] until the peer acknowledges it (`message_delivered` with that id).
 * [queued]: there was no link when it was sent, so the core holds it until
 * the device is connected. [failed]: the send failed. Incoming ones carry the
 * sender’s id in [msgId] (one made here when it gave none). Older ones come from
 * [ChatHistory] ([fromHistory]: no live state, so no "sending" marker).
 */
data class ChatMessage(
    val id: Long, val node: String, val incoming: Boolean, val text: String, val ts: Long,
    val msgId: String? = null,
    val queued: Boolean = false,
    val delivered: Boolean = false,
    val failed: Boolean = false,
    val fromHistory: Boolean = false,
) {
    val pending: Boolean get() = !incoming && !delivered && !failed && !fromHistory
}

enum class Tone { Info, Ok, Warn, Error }

data class ActivityItem(val id: Long, val ts: Long, val text: String, val tone: Tone)

/** One recent clipboard value. [source]: the device it came from (null =
 *  this phone). Kept in memory only: it may hold passwords or codes. */
data class ClipEntry(val text: String, val source: String?, val ts: Long)

data class AppEntry(val pkg: String, val label: String)

data class AppsState(val apps: List<AppEntry>, val subscribed: Set<String>)

/** A device's reported status (`device_status` event); fields it didn't
 *  report are null (a desktop has no battery). */
/** What a device reported about itself: battery, the actions it takes
 *  (flashlight, volume, media, ringer, mute) and their current values. */
data class DeviceStatus(
    val battery: Int?, val charging: Boolean?,
    val actions: Set<String> = emptySet(),
    val torch: Boolean? = null, val volume: Int? = null, val ringer: String? = null, val muted: Boolean? = null,
    /** Degrees Celsius, and "good" / "overheat" / "cold" / ... (phones). */
    val batteryTemp: Int? = null, val batteryHealth: String? = null,
    val wifiOn: Boolean? = null, val wifiConnected: Boolean? = null, val ssid: String? = null, val wifiSignal: Int? = null,
    val bluetooth: Boolean? = null,
    val ramFree: Long? = null, val ramTotal: Long? = null,
    /** Why a computer has no slide remote: "accessibility" (macOS
     *  permission), "xdotool" or "wtype" (tool to install). */
    val slidesNeeds: String? = null,
    /** Why a computer's touchpad can't work yet: "accessibility" (macOS
     *  permission) or "display" (no X11 session). */
    val inputNeeds: String? = null,
    /** Why a computer's screen can't be seen yet: "screen_recording" (macOS
     *  permission), "grim" or "imagemagick" (tool to install). */
    val screenNeeds: String? = null,
    val storageFree: Long? = null, val storageTotal: Long? = null,
    /** It lets your other devices see where it is (its own switch). */
    val locationSharing: Boolean = false,
    /** Where it last said it was (after a Locate). */
    val location: Place? = null,
    /** How many folders it lets your devices browse (null: it can't). */
    val sharedFolders: Int? = null,
    /** Percent of its processor in use lately, 0-100 (computers only). */
    val cpu: Int? = null,
    /** Seconds since it started, when it answered (at [seenAt], this
     *  phone's clock in ms). */
    val uptimeS: Long? = null,
    /** Commands its owner lets your devices run there (the `run` action). */
    val commands: List<RemoteCommand> = emptyList(),
    /** How the last command run there went. */
    val lastRun: LastRun? = null,
    /** It’s in Do Not Disturb (phones; null: it didn’t say). */
    val dnd: Boolean? = null,
    /** A computer this phone may share its screen with from here
     *  (`screen_view`; older ones would show the offer as chat). */
    val screenView: Boolean = false,
    /** A computer that plays this phone's audio (`audio_sink`; older ones
     *  would show the messages as chat). */
    val audioSink: Boolean = false,
    /** A computer that plays its sound on this phone (`audio_source`;
     *  older ones only play a phone's). */
    val audioSource: Boolean = false,
    /** A computer that can show its own notifications on this phone
     *  (`notif_share`: whether its switch is on; null: it can't, or is older). */
    val notifShare: Boolean? = null,
    /** "android" or "desktop" (null: it didn't say). */
    val platform: String? = null,
    /** It takes media commands (`media_ctl`): play, pause, skip, seek and
     *  volume from here. */
    val mediaCtl: Boolean = false,
    val seenAt: Long = System.currentTimeMillis(),
)

/** A command a computer lets your devices run: only its [id] is sent. */
data class RemoteCommand(val id: String, val name: String)

/** How a command run on a computer went: its exit [code] (null when it was
 *  stopped), the end of its [output], or why it was refused ([error]); [at]
 *  is unix seconds on that computer's clock. */
data class LastRun(
    val id: String, val name: String, val code: Int?, val output: String,
    val timedOut: Boolean, val error: String?, val at: Long,
)

/** A reported position: degrees, accuracy in metres, unix seconds. */
data class Place(val lat: Double, val lon: Double, val acc: Double, val at: Long)

/** One item in another device's shared folder: a file ([size] bytes) or a
 *  folder ([dir]), last changed at [modified] (unix seconds). */
data class RemoteFile(val name: String, val dir: Boolean, val size: Long, val modified: Long)

/** One page of a folder on another device (`folder_listing`, the answer to
 *  [Core.listFolder]). [root] "" is the list of its shared folders; [next]
 *  is the offset of the following page; [error] says why there's nothing. */
data class FolderListing(
    val node: String, val root: String, val path: String,
    val entries: List<RemoteFile>, val next: Long?, val error: String?,
)

/** What's playing on a device (`now_playing`, also in its status). Times
 *  in ms; [at] is when [position] was true (this phone's clock). An older
 *  app sends only the title, artist, app and whether it plays. */
data class Playing(
    val title: String, val artist: String?, val app: String?, val playing: Boolean,
    val duration: Long? = null, val position: Long? = null, val at: Long = System.currentTimeMillis(),
    val canNext: Boolean = true, val canPrev: Boolean = true, val canSeek: Boolean = false,
    val volume: Int? = null, val volumeMax: Int? = null,
    /** The artwork as base64 JPEG/PNG. */
    val art: String? = null,
    val session: String? = null,
    /** Other apps playing there: (id, label). */
    val sessions: List<Pair<String, String>> = emptyList(),
) {
    /** Where the track is now (it runs on while playing). */
    fun positionNow(now: Long = System.currentTimeMillis()): Long? =
        MediaPlan.positionNow(position, duration, playing, now - at)
}

private fun playingOf(o: JSONObject?): Playing? {
    val title = o?.optString("title").orEmpty()
    if (o == null || title.isEmpty()) return null
    val state = o.optString("state")
    if (state != "playing" && state != "paused") return null
    fun long(k: String) = if (o.has(k) && !o.isNull(k)) o.optLong(k).takeIf { it >= 0 } else null
    val vmax = long("volume_max")?.toInt()?.takeIf { it > 0 }
    val list = o.optJSONArray("sessions")
    val sessions = (0 until (list?.length() ?: 0)).mapNotNull { i ->
        val s = list!!.optJSONObject(i) ?: return@mapNotNull null
        val id = s.optString("id").ifEmpty { return@mapNotNull null }
        id to (s.optString("app").ifEmpty { id } + s.optString("title").let { if (it.isEmpty()) "" else " · $it" })
    }
    return Playing(
        title,
        o.optString("artist").ifEmpty { null },
        o.optString("app").ifEmpty { null },
        state == "playing",
        duration = long("duration")?.takeIf { it > 0 },
        position = long("position"),
        canNext = o.optBoolean("can_next", true),
        canPrev = o.optBoolean("can_prev", true),
        canSeek = o.optBoolean("can_seek", false),
        volume = if (vmax != null) long("volume")?.toInt() else null,
        volumeMax = vmax,
        art = o.optString("art").ifEmpty { null },
        session = o.optString("session").ifEmpty { null },
        sessions = sessions,
    )
}

/** One raw core event, for the Developer screen. */
data class DevEvent(val ts: Long, val name: String, val payload: String)

/**
 * The app's view of the core: observable state kept current by core events,
 * plus suspend wrappers over [MdEngine]. Screens read the state and call the
 * actions here; they never hold the engine themselves.
 *
 * Mirrors `apps/desktop/ui/src/lib/core.ts`, so both apps behave alike.
 */
object Core {
    private val main = Handler(Looper.getMainLooper())
    private var seq = 0L

    var me by mutableStateOf<DeviceIdentity?>(null); private set
    var session by mutableStateOf<Session?>(null); private set
    var ready by mutableStateOf(false); private set
    var peers by mutableStateOf<List<PeerInfo>>(emptyList()); private set

    /** node id -> live link transport ("quic", "relay", ...). */
    val links = mutableStateMapOf<String, String>()

    /** node id -> latest connect-ladder stage. */
    val progress = mutableStateMapOf<String, String>()

    /** node id -> measured link quality ("excellent", "good", "weak"). */
    val quality = mutableStateMapOf<String, String>()
    val messages = mutableStateListOf<ChatMessage>()
    val activity = mutableStateListOf<ActivityItem>()
    val apps = mutableStateMapOf<String, AppsState>()

    /** The latest snapshot of each computer screen being watched. */
    val screens = mutableStateMapOf<String, ImageBitmap>()
    /** node id -> how a media stream with it stands (nearby media mode:
     *  the "Nearby · low latency" badge and the latency). */
    val nearby = mutableStateMapOf<String, NearbyBadge.State>()
    val devLog = mutableStateListOf<DevEvent>()

    /** node id -> what's playing there (absent: nothing, or not asked). */
    val nowPlaying = mutableStateMapOf<String, Playing>()

    /** node id -> its latest reported status (battery, charging). */
    val deviceStatus = mutableStateMapOf<String, DeviceStatus>()

    /** File transfers, newest first (this process only). */
    val transfers = mutableStateListOf<Transfer>()

    /** Files shared to the app from another app (share sheet), waiting for
     *  the user to pick a device. */
    var pendingShare by mutableStateOf<List<Uri>>(emptyList()); private set
    /** Ask the UI to show the Settings tab (from a notification). */
    var showSettings by mutableStateOf(false)

    /** Ask the UI to open this device's screen (a launcher shortcut). */
    var showDevice by mutableStateOf<String?>(null)

    /** Ask the UI to open this note (a reminder's notification). */
    var showNote by mutableStateOf<String?>(null)

    /** Ask the UI to open Timers (a timer's notification). */
    var showTimers by mutableStateOf(false)

    /** A scanned `mydomain://join?…` link (the camera or Lens opened it),
     *  waiting for the sign-in screen to join with it. */
    var pendingJoin by mutableStateOf<String?>(null)

    /** Text or a link shared to the app from another app, waiting for the
     *  user to pick a device (files win if both came). */
    var pendingText by mutableStateOf<String?>(null); private set

    /** Answers to [listFolder], by request id (this process only). */
    val listings = mutableStateMapOf<String, FolderListing>()

    /** Clipboard auto-sync with the other devices (NetService acts on it). */
    var clipSync by mutableStateOf(false); private set

    /** Where this phone's photo backup stands (`backup_state` events). */
    var backup by mutableStateOf<com.mydomain.android.ffi.BackupInfo?>(null); private set

    /** Synced folders on this phone, invitations included (`sync_pairs`
     *  events). */
    val syncPairs = mutableStateListOf<com.mydomain.android.ffi.SyncPairInfo>()

    /** Recent clipboard values, newest first (memory only, never saved). */
    val clipHistory = mutableStateListOf<ClipEntry>()

    /** Text the app itself just put on the system clipboard from
     *  [clipHistory]; the clipboard listener skips recording it once. */
    @Volatile private var recopied: String? = null

    fun engine(): MdEngine? = runningEngine()

    /** Events that change what the home-screen widget shows. */
    private val WIDGET_KINDS = setOf("peers_updated", "peer_connected", "peer_disconnected", "device_status")

    /** Events that change the share sheet's device targets (which devices,
     *  and which have a live link: those rank first). */
    private val SHARE_TARGET_KINDS = setOf("peers_updated", "peer_connected", "peer_disconnected", "device_kind")
    /** Events after which "How your devices are connected" reads its map again. */
    private val MAP_KINDS = setOf("peers_updated", "peer_connected", "peer_disconnected", "links_map_changed", "labels_changed", "nearby_media", "device_kind")

    /** The app context, for strings built outside the screens (set by
     *  MainActivity and NetService, whichever starts first). */
    lateinit var app: Context
        private set

    fun init(context: Context) {
        app = context.applicationContext
        BatteryLog.load(app)
        ClipLog.load(app)
        ChatHistory.load(app)
        DeviceLabels.load(app)
        PageLayoutStore.load(app)
        TransferLog.load(app)
        ActivityLog.load(app)
    }

    fun str(@StringRes id: Int, vararg args: Any): String = app.getString(id, *args)

    fun plural(@PluralsRes id: Int, n: Int, vararg args: Any): String =
        app.resources.getQuantityString(id, n, *args)

    /** Is the app on screen? Off screen, the engine closes idle links
     *  (battery); a send re-opens them and pushes still arrive. */
    @Volatile var visible = false
        private set

    /** Activities between onStart and onStop. More than one can be: a
     *  share or `am start` opens a second MainActivity, and the first one
     *  stops after the new one starts. */
    private var started = 0

    /** An activity started (`true`) or stopped (`false`). */
    fun setVisible(on: Boolean) {
        val now = synchronized(this) {
            started = (started + if (on) 1 else -1).coerceAtLeast(0)
            val now = started > 0
            if (now == visible) return
            visible = now
            now
        }
        labLog(if (now) "app visible" else "app in background")
        onMain { onScreen = now }
        runCatching { onVisibility?.invoke(now) }
        Thread { engine()?.setBackground(!now) }.start()
    }

    /** [visible], for the screens (main thread): their timers and polls
     *  wait while the app is off screen ([awaitOnScreen]). */
    var onScreen by mutableStateOf(false)
        private set

    /** Told when the app comes on or goes off screen (NetService: the
     *  multicast lock). Any thread. */
    @Volatile var onVisibility: ((Boolean) -> Unit)? = null

    /** Return at once while the app is on screen, else when it comes back.
     *  Each screen's repeating work (polls, clocks, watches) calls this
     *  before each round: a screen left open behind the home screen keeps
     *  its composition, and its loops would otherwise keep running. */
    suspend fun awaitOnScreen() {
        if (onScreen) return
        androidx.compose.runtime.snapshotFlow { onScreen }.first { it }
    }

    /** The name to show for a device: the nickname you gave it on this
     *  phone ([DeviceLabels]), else its own name tidied. Screens, tiles,
     *  the widget and notifications all name devices through this. */
    fun peerName(node: String?): String =
        DeviceLabels.nickname(node)
            ?: peers.firstOrNull { it.nodeId == node }?.name?.let(::niceName)
            ?: str(R.string.a_device)

    /** Give a device a nickname, an icon and a colour on this phone (blank:
     *  none). Main thread. */
    fun setDeviceLabel(node: String, nickname: String?, icon: String?, color: String?) {
        val label = DeviceLabels.set(app, node, nickname, icon, color)
        HomeWidget.refresh(app)
        ShareTargets.refresh(app)
        // Your other devices see it too, while sharing is on.
        Thread {
            runCatching { engine()?.setSharedLabel(node, label.nickname, label.icon, label.color) }
        }.start()
    }

    /** Whether device labels are shared with your other devices (on until
     *  turned off). */
    fun labelsShare(): Boolean = runCatching { engine()?.labelsShare() }.getOrNull() ?: true

    fun setLabelsShare(on: Boolean) {
        Thread { runCatching { engine()?.setLabelsShare(on) } }.start()
    }

    internal fun onMain(block: () -> Unit) {
        if (Looper.myLooper() == Looper.getMainLooper()) block() else main.post(block)
    }

    fun addActivity(text: String, tone: Tone = Tone.Info) = onMain {
        activity.add(0, ActivityItem(++seq, System.currentTimeMillis(), text, tone))
        if (activity.size > 300) activity.removeRange(300, activity.size)
    }

    // --- state from the engine ---------------------------------------------

    /** Re-read everything the screens show. Safe from any thread. */
    fun reload() {
        val e = engine() ?: return
        val id = e.identity()
        val s = e.session()
        val r = e.isReady()
        val p = e.peers()
        onMain {
            me = id
            session = s
            ready = r
            peers = p
            links.clear()
            p.forEach { peer -> peer.link?.let { links[peer.nodeId] = it } }
            HomeWidget.refresh(app)
            ShareTargets.refresh(app)
        }
        Notes.refresh()
        Timers.refresh()
        ReadLater.refresh()
    }

    /** Re-read every live link's quality (called every few seconds while
     *  the app is on screen). */
    fun pollQuality() {
        val stats = engine()?.linkStats() ?: return
        onMain {
            quality.clear()
            stats.forEach { quality[it.nodeId] = it.quality }
        }
    }

    fun enableClipSync(on: Boolean) = onMain { clipSync = on }

    /** Remember a clipboard value ([source] null = copied on this phone).
     *  A repeat moves to the top; the newest 20 are kept. Safe from any thread. */
    fun recordClip(text: String, source: String?) = onMain {
        if (text.isBlank()) return@onMain
        clipHistory.removeAll { it.text == text }
        clipHistory.add(0, ClipEntry(text, source, System.currentTimeMillis()))
        if (clipHistory.size > 20) clipHistory.removeRange(20, clipHistory.size)
    }

    fun clearClipHistory() = onMain { clipHistory.clear() }

    /** Put a history entry back on the system clipboard. Moves it to the top
     *  (keeping its source) without recording it again as a local copy. */
    fun copyFromHistory(entry: ClipEntry) {
        recopied = entry.text
        val cm = app.getSystemService(Context.CLIPBOARD_SERVICE) as android.content.ClipboardManager
        cm.setPrimaryClip(android.content.ClipData.newPlainText("Clipboard", entry.text))
        recordClip(entry.text, entry.source)
    }

    /** True once if [text] is what [copyFromHistory] just set. */
    fun consumeRecopy(text: String): Boolean {
        if (recopied != text) return false
        recopied = null
        return true
    }

    // --- events --------------------------------------------------------------

    /** Debug builds: where `labLog` appends (set by NetService). Some phones
     *  (vivo) rate-limit logcat and mute a chatty app, so the lab reads this
     *  file instead: `adb shell run-as com.mydomain.android cat files/events.log`. */
    @Volatile var labLogFile: java.io.File? = null

    /** Debug builds: one timestamped line in logcat and in `labLogFile`
     *  (rotated at 256 KB). */
    fun labLog(line: String) {
        if (!BuildConfig.DEBUG) return
        Log.i("mydomain_event", line)
        val f = labLogFile ?: return
        runCatching {
            synchronized(this) {
                if (f.length() > 256 * 1024) f.renameTo(java.io.File(f.path + ".1"))
                val ts = java.text.SimpleDateFormat("HH:mm:ss.SSS", java.util.Locale.US).format(java.util.Date())
                f.appendText("$ts $line\n")
            }
        }
    }

    /** Every core event, from NetService's listener (core threads). */
    fun onEvent(name: String, payloadJson: String) {
        // A screen snapshot: decoded here, off the main thread, and kept out
        // of the logs (it's a big picture, twice a second).
        if (name == "screen-frame") {
            val o = runCatching { JSONObject(payloadJson) }.getOrNull() ?: return
            val bytes = runCatching { android.util.Base64.decode(o.optString("jpeg"), android.util.Base64.DEFAULT) }.getOrNull() ?: return
            val bmp = android.graphics.BitmapFactory.decodeByteArray(bytes, 0, bytes.size)?.asImageBitmap() ?: return
            val from = o.optString("from")
            onMain { screens[from] = bmp }
            return
        }
        // Wi-Fi Direct / hotspot requests carry a passphrase: handled here,
        // never logged or kept.
        if (name == "wifi-direct-request") {
            WifiDirect.onRequest(app, payloadJson)
            return
        }
        // A code event's payload holds the code itself: never log it.
        labLog(if (name.startsWith("code")) name else "$name ${payloadJson.take(200)}")
        val o = runCatching { JSONObject(payloadJson) }.getOrNull() ?: return
        val kind = o.optString("kind")
        val peersNow = if (kind == "peers_updated") engine()?.peers() else null
        if (kind == "backup_state") engine()?.let { e -> runCatching { e.backupState() }.getOrNull()?.let { s -> onMain { backup = s } } }
        if (kind == "sync_pairs") refreshSync()
        // An outgoing transfer ended: its cache copy can go (a folder's, once
        // the whole folder is done).
        if (kind == "file_sent" || (kind == "file_failed" && !o.optBoolean("incoming"))) {
            OutgoingFiles.end(o.optString("id"))
            OutgoingFolders.transferEnded()
        }
        onMain {
            devLog.add(0, DevEvent(System.currentTimeMillis(), name, payloadJson))
            if (devLog.size > 500) devLog.removeRange(500, devLog.size)
            apply(o, peersNow)
            if (kind in WIDGET_KINDS) HomeWidget.refresh(app)
            if (kind in SHARE_TARGET_KINDS) ShareTargets.refresh(app)
            if (kind in MAP_KINDS) LinksMap.changed()
        }
    }

    private fun apply(o: JSONObject, peersNow: List<PeerInfo>?) {
        val node = o.optString("node_id")
        // What the Activity timeline keeps that no history does.
        ActivityLog.onEvent(o)
        when (o.optString("kind")) {
            "peers_updated" -> peersNow?.let { peers = it }
            // Labels set on your other devices: take them here.
            "labels_changed" -> {
                DeviceLabels.applyShared(app, o.optJSONArray("labels"))
                HomeWidget.refresh(app)
                ShareTargets.refresh(app)
            }
            // A device said what kind it is (its icon).
            "device_kind" -> DeviceKinds.apply(o)
            // Notes changed on your other devices (and ones deleted there).
            "notes_changed" -> Notes.applyChanged(o)
            // Timers started or changed on your other devices (every timer shown).
            "timers_changed" -> Timers.applyChanged(o)
            // Links saved to read later changed on your other devices.
            "read_later_changed" -> ReadLater.applyChanged(o)
            // A link downloaded here or on another device ("Download on…"):
            // its transfer row says the link's host.
            "remote_download" -> RemoteDownloads.onEvent(o)?.let { d ->
                updateTransfer(d.id, { Transfer(it, d.node, d.name, d.here) }) {
                    it.copy(name = d.name.ifEmpty { it.name }, link = d.host)
                }
            }
            "peer_connected" -> {
                val was = links.containsKey(node)
                links[node] = o.optString("transport")
                progress[node] = "connected"
                if (!was) addActivity(str(R.string.act_connected, peerName(node)), Tone.Ok)
            }
            "peer_disconnected" -> {
                links.remove(node)
                progress.remove(node)
                addActivity(str(R.string.act_disconnected, peerName(node)))
            }
            "connect_progress" -> progress[node] = o.optString("stage")
            // A speed test from a computer's Connection tile.
            "speed_test" -> ConnCheck.onSpeedEvent(o)
            "message_delivered" -> {
                val id = o.optString("id")
                messages.replaceAll { if (!it.incoming && it.msgId == id) it.copy(delivered = true) else it }
                ChatHistory.delivered(id)
            }
            "message_received" -> {
                if (!o.optBoolean("ok", true)) {
                    addActivity(str(R.string.act_unreadable), Tone.Error)
                    return
                }
                val text = o.optString("text")
                if (ClipboardSync.isControl(text)) return
                val from = node.ifEmpty { peers.firstOrNull { it.ip == o.optString("ip") }?.nodeId ?: "" }
                // One without an id (an old sender) gets one here, the same in the history.
                val msgId = o.optString("id").ifEmpty { java.util.UUID.randomUUID().toString() }
                messages.add(ChatMessage(++seq, from, true, text, System.currentTimeMillis(), msgId))
                if (from.isNotEmpty()) ChatHistory.received(from, msgId, text)
                addActivity(str(R.string.act_message, peerName(from), text))
            }
            "notification" -> {
                val title = o.optString("title")
                val body = o.optString("body")
                addActivity(if (body.isNotEmpty()) str(R.string.act_notification, title, body) else title)
            }
            "call_notification" -> addActivity(str(R.string.act_call, o.optString("caller"), o.optString("state")))
            "call_history" -> addActivity(str(R.string.act_call_history))
            "now_playing" -> {
                val from = o.optString("from")
                playingOf(o.optJSONObject("media"))?.let { nowPlaying[from] = it } ?: nowPlaying.remove(from)
            }
            "device_status" -> {
                val st = o.optJSONObject("status") ?: JSONObject()
                val from = o.optString("from")
                playingOf(st.optJSONObject("now_playing"))?.let { nowPlaying[from] = it } ?: nowPlaying.remove(from)
                val acts = st.optJSONArray("actions")
                // A sample for its battery graph (InfoTile).
                if (st.has("battery")) BatteryLog.record(from, st.optInt("battery"), st.optBoolean("charging", false))
                deviceStatus[o.optString("from")] = DeviceStatus(
                    battery = if (st.has("battery")) st.optInt("battery") else null,
                    charging = if (st.has("charging")) st.optBoolean("charging") else null,
                    actions = (0 until (acts?.length() ?: 0)).map { acts!!.optString(it) }.toSet(),
                    torch = if (st.has("torch")) st.optBoolean("torch") else null,
                    volume = if (st.has("volume")) st.optInt("volume") else null,
                    ringer = st.optString("ringer").ifEmpty { null },
                    muted = if (st.has("muted")) st.optBoolean("muted") else null,
                    batteryTemp = if (st.has("battery_temp")) st.optInt("battery_temp") else null,
                    batteryHealth = st.optString("battery_health").ifEmpty { null },
                    screenView = st.optBoolean("screen_view", false),
                    audioSink = st.optBoolean("audio_sink", false),
                    audioSource = st.optBoolean("audio_source", false),
                    platform = st.optString("platform").ifEmpty { null },
                    mediaCtl = st.optInt("media_ctl", 0) >= 1,
                    notifShare = if (st.has("notif_share")) st.optBoolean("notif_share") else null,
                    wifiOn = st.optJSONObject("wifi")?.let { if (it.has("on")) it.optBoolean("on") else null },
                    wifiConnected = st.optJSONObject("wifi")?.let { if (it.has("connected")) it.optBoolean("connected") else null },
                    ssid = st.optJSONObject("wifi")?.optString("ssid")?.ifEmpty { null },
                    wifiSignal = st.optJSONObject("wifi")?.let { if (it.has("signal")) it.optInt("signal") else null },
                    bluetooth = if (st.has("bluetooth")) st.optBoolean("bluetooth") else null,
                    slidesNeeds = st.optString("slides_needs").ifEmpty { null },
                    inputNeeds = st.optString("input_needs").ifEmpty { null },
                    screenNeeds = st.optString("screen_needs").ifEmpty { null },
                    ramFree = st.optJSONObject("ram")?.optLong("free"),
                    ramTotal = st.optJSONObject("ram")?.optLong("total"),
                    storageFree = st.optJSONObject("storage")?.optLong("free"),
                    storageTotal = st.optJSONObject("storage")?.optLong("total"),
                    locationSharing = st.optBoolean("location_sharing", false),
                    sharedFolders = if (st.has("shared_folders")) st.optInt("shared_folders") else null,
                    cpu = if (st.has("cpu")) st.optInt("cpu").coerceIn(0, 100) else null,
                    uptimeS = if (st.has("uptime_s")) st.optLong("uptime_s").coerceAtLeast(0) else null,
                    dnd = if (st.has("dnd")) st.optBoolean("dnd") else null,
                    commands = st.optJSONArray("commands")?.let { arr ->
                        (0 until arr.length()).mapNotNull { i ->
                            arr.optJSONObject(i)?.takeIf { it.has("id") && it.has("name") }
                                ?.let { RemoteCommand(it.optString("id"), it.optString("name")) }
                        }
                    } ?: emptyList(),
                    lastRun = st.optJSONObject("last_run")?.takeIf { it.has("id") && it.has("at") }?.let {
                        LastRun(
                            id = it.optString("id"),
                            name = it.optString("name"),
                            code = if (it.has("code") && !it.isNull("code")) it.optInt("code") else null,
                            output = it.optString("output"),
                            timedOut = it.optBoolean("timed_out", false),
                            error = if (it.isNull("error")) null else it.optString("error").ifEmpty { null },
                            at = it.optLong("at"),
                        )
                    },
                    location = st.optJSONObject("location")?.takeIf { it.has("lat") && it.has("lon") }?.let {
                        Place(it.optDouble("lat"), it.optDouble("lon"), it.optDouble("acc", 0.0), it.optLong("at"))
                    },
                )
            }
            "ring" -> addActivity(str(R.string.act_ring, peerName(o.optString("from"))), Tone.Warn)
            // A media stream moved to (or off) a local link, or its latency.
            "nearby_media" -> nearby[node] = nearbyState(o)
            // A device came back with another security key: refused until
            // the user decides (its screen asks).
            "key_changed" -> {
                addActivity(str(R.string.key_change_title, peerName(node)), Tone.Warn)
                appScope.launch { runCatching { KeyChanges.load() } }
            }
            "battery_alert" -> {
                val who = peerName(o.optString("from"))
                when (o.optString("alert")) {
                    "low" -> addActivity(str(R.string.act_battery_low, who, o.optInt("level")), Tone.Warn)
                    "charged" -> addActivity(str(R.string.act_battery_charged, who, o.optInt("level")), Tone.Ok)
                    else -> addActivity(str(R.string.act_battery_full, who), Tone.Ok)
                }
            }
            "open_url" -> addActivity(str(R.string.act_link, peerName(o.optString("from")), o.optString("url")))
            "clipboard" -> addActivity(str(R.string.act_clipboard_synced))
            "text_shared" -> {
                // Marked sensitive (a password): neither kept nor shown.
                val secret = o.optBoolean("sensitive")
                if (o.optString("mode") == "clipboard" && !secret) ClipLog.received(o.optString("from"), o.optString("text"))
                val shown = if (secret) str(R.string.text_password_hidden) else o.optString("text").take(200)
                addActivity(
                    str(if (o.optString("mode") == "clipboard") R.string.act_text_copied else R.string.act_text_from, peerName(o.optString("from")), shown),
                )
            }
            "folder_listing" -> {
                val arr = o.optJSONArray("entries") ?: JSONArray()
                val entries = (0 until arr.length()).mapNotNull { i ->
                    val e = arr.optJSONObject(i) ?: return@mapNotNull null
                    RemoteFile(e.optString("name"), e.optBoolean("dir"), e.optLong("size"), e.optLong("modified"))
                }
                val req = o.optString("req")
                listings[req] = FolderListing(
                    node = o.optString("from"), root = o.optString("root"), path = o.optString("path"),
                    entries = entries,
                    next = if (o.isNull("next")) null else o.optLong("next"),
                    error = if (o.isNull("error")) null else o.optString("error"),
                )
                if (listings.size > 50) listings.keys.firstOrNull { it != req }?.let { listings.remove(it) }
            }
            "file_progress" -> {
                updateTransfer(o.optString("id"), { Transfer(it, node, o.optString("name"), o.optBoolean("incoming")) }) {
                    if (it.state != TransferState.Active) it
                    else it.copy(done = o.optLong("done"), total = o.optLong("total"))
                }
                TransferLog.progress(
                    o.optString("id"), node, o.optString("name"), o.optLong("done"), o.optLong("total"),
                    o.optBoolean("incoming"), o.optBoolean("resumed"),
                    if (o.isNull("batch")) null else o.optString("batch").ifEmpty { null },
                )
            }
            // Paused or resumed, here or on the other device.
            "file_paused" -> TransferLog.paused(o.optString("id"), o.optBoolean("paused"))
            // A folder or several files sent together: one row, and one
            // line in the activity once they're all in.
            "transfer_batch" -> {
                val item = o.optJSONObject("item") ?: return
                val id = item.optString("id")
                val before = TransferLog.list.firstOrNull { it.id == id }?.state
                TransferLog.batch(item)
                if (item.optString("state") == "done" && before != TransferState.Done && item.optBoolean("incoming")) {
                    val from = item.optString("node_id")
                    val text = if (item.optString("kind") == "folder") {
                        str(R.string.act_batch_saved_folder, item.optString("name"), peerName(from))
                    } else {
                        str(R.string.act_batch_saved_files, item.optInt("count"), peerName(from))
                    }
                    addActivity(text, Tone.Ok)
                }
            }
            // Transfers with a device went over to Wi-Fi Direct or the
            // phone's hotspot (or back).
            "wifi_direct" -> TransferLog.wifiDirect(node, o.optBoolean("active"), o.optString("via", TransferBook.VIA_WIFI_DIRECT))
            "file_received" -> {
                val from = o.optString("from")
                val name = o.optString("name")
                TransferLog.received(o.optString("id"), from, name, o.optLong("size"))
                updateTransfer(o.optString("id"), { Transfer(it, from, name, true) }) {
                    it.copy(state = TransferState.Done, done = o.optLong("size"), total = o.optLong("size"))
                }
                // A synced folder's files come by themselves, often many; a
                // batch's are told about with the batch.
                if (!o.optBoolean("sync") && !TransferLog.inBatch(o.optString("id"))) {
                    addActivity(str(R.string.act_file_saved, name, peerName(from)), Tone.Ok)
                }
            }
            "file_sent" -> {
                val name = o.optString("name")
                TransferLog.sent(o.optString("id"), node, name)
                updateTransfer(o.optString("id"), { Transfer(it, node, name, false) }) {
                    it.copy(state = TransferState.Done, done = it.total)
                }
                if (!TransferLog.inBatch(o.optString("id"))) addActivity(str(R.string.act_file_sent, name, peerName(node)), Tone.Ok)
            }
            "file_failed" -> {
                val name = o.optString("name")
                val incoming = o.optBoolean("incoming")
                val reason = o.optString("reason")
                TransferLog.failed(o.optString("id"), node, name, incoming, reason)
                updateTransfer(o.optString("id"), { Transfer(it, node, name, incoming) }) {
                    it.copy(state = TransferState.Failed, reason = reason)
                }
                if (reason != "cancelled") {
                    addActivity(str(if (incoming) R.string.act_file_receive_failed else R.string.act_file_send_failed, name, peerName(node), reason), Tone.Error)
                }
            }
            "apps_list" -> {
                val from = o.optString("from")
                val key = peers.firstOrNull { it.nodeId == from || it.name == from }?.nodeId ?: from
                val arr = o.optJSONArray("apps") ?: JSONArray()
                val list = (0 until arr.length()).map {
                    val a = arr.getJSONObject(it)
                    AppEntry(a.optString("pkg"), a.optString("label").ifBlank { a.optString("pkg") })
                }
                val sub = o.optJSONArray("subscribed") ?: JSONArray()
                apps[key] = AppsState(list, (0 until sub.length()).map { sub.getString(it) }.toSet())
            }
        }
    }

    /** Update transfer [id] (main thread), creating it first if unknown. */
    private fun updateTransfer(id: String, create: (String) -> Transfer, change: (Transfer) -> Transfer) {
        if (id.isEmpty()) return
        val i = transfers.indexOfFirst { it.id == id }
        if (i < 0) {
            transfers.add(0, change(create(id)))
            if (transfers.size > 100) transfers.removeRange(100, transfers.size)
        } else {
            transfers[i] = change(transfers[i])
        }
    }

    /** NetService moved received file [id] into Downloads: it can be opened. */
    fun fileSaved(id: String, uri: Uri, mime: String) = onMain {
        val i = transfers.indexOfFirst { it.id == id }
        if (i >= 0) transfers[i] = transfers[i].copy(uri = uri.toString(), mime = mime)
        TransferLog.saved(id, uri.toString(), mime)
    }

    /** Files shared from another app; the UI asks which device gets them. */
    fun shareFiles(uris: List<Uri>) = onMain { pendingShare = uris }

    /** Text shared from another app; the UI asks which device gets it. */
    fun shareText(text: String) = onMain { pendingText = text }

    fun clearShare() = onMain { pendingShare = emptyList(); pendingText = null }

    // --- actions -------------------------------------------------------------

    /** Run [block] on the engine off the main thread; core errors are thrown. */
    suspend fun <T> call(block: (MdEngine) -> T): T = withContext(Dispatchers.IO) {
        val e = engine() ?: throw IllegalStateException("not started")
        block(e)
    }

    /** Join the account another device invites to, with its 6-digit code
     *  (on [server]) or a scanned join link: signed in with that device's
     *  key, no password. */
    suspend fun joinAccount(server: String, codeOrLink: String) {
        withContext(Dispatchers.IO) {
            var e = engine()
            repeat(50) { if (e == null) { Thread.sleep(100); e = engine() } }
            val engine = e ?: throw IllegalStateException("not started")
            val v = codeOrLink.trim()
            if (JoinLinks.isJoinLink(v)) engine.joinWithLink(v) else engine.joinWithCode(server, v)
            runCatching { engine.refresh() }
        }
        reload()
        // First-run setup opens once (the app shell watches this).
        Health.justSignedIn = true
    }

    /** "Add a device": a one-time QR code and 6-digit code (5 minutes). */
    suspend fun joinInvite() = call { it.joinInvite() }

    /** Has the invited device joined ("waiting", "joined" + name, "none")? */
    suspend fun joinStatus() = call { it.joinStatus() }

    /** The invite was closed: withdraw it. */
    suspend fun joinCancel() = call { it.joinCancel() }

    /** Sign in (or create the account), set the device key, load devices. */
    suspend fun signIn(server: String, user: String, pass: String, key: String, create: Boolean) {
        withContext(Dispatchers.IO) {
            // NetService starts the engine; on a fresh launch it may not be up yet.
            var e = engine()
            repeat(50) { if (e == null) { Thread.sleep(100); e = engine() } }
            val engine = e ?: throw IllegalStateException("not started")
            if (create) engine.authRegister(server, user, pass) else engine.authLogin(server, user, pass)
            engine.setEncryptionKey(key)
            runCatching { engine.refresh() }
        }
        reload()
        Health.justSignedIn = true
    }

    suspend fun refreshDevices() {
        call { it.refresh() }
        reload()
    }

    /**
     * Send a chat message; returns the transport used ("queued" with no link
     * yet). The bubble is added first, so a fast `message_delivered` finds it.
     */
    suspend fun sendMessage(node: String, text: String): String {
        val msgId = java.util.UUID.randomUUID().toString()
        onMain { messages.add(ChatMessage(++seq, node, false, text, System.currentTimeMillis(), msgId)) }
        // Kept before it goes, so a fast receipt finds it in the history.
        ChatHistory.sending(node, msgId, text)
        val via = try {
            call { it.sendWithId(node, text, msgId) }
        } catch (e: Exception) {
            // Kept (marked) as it is in the history.
            onMain { messages.replaceAll { if (it.msgId == msgId) it.copy(failed = true) else it } }
            ChatHistory.sendResult(msgId, ChatStatus.Failed)
            throw e
        }
        ChatHistory.sendResult(msgId, if (via == "queued") ChatStatus.Queued else ChatStatus.Sent)
        if (via == "queued") {
            onMain { messages.replaceAll { if (it.msgId == msgId) it.copy(queued = true) else it } }
        }
        return via
    }

    suspend fun connect(node: String) {
        onMain { progress[node] = "quic" }
        call { it.connect(node) }
    }

    /** Close the link now; it comes back when either side sends something. */
    suspend fun disconnect(node: String) {
        call { it.disconnect(node) }
    }

    /** Close the link and connect afresh (for a link that's up but stuck). */
    suspend fun reconnect(node: String) {
        onMain { progress[node] = "quic" }
        call { it.reconnect(node) }
    }

    /** Forget the chat with [node], on this phone only (its saved history
     *  too). */
    fun clearChat(node: String) {
        messages.removeAll { it.node == node }
        ChatHistory.clear(node)
    }

    /** Ask a device for its clipboard; NetService applies the reply. */
    suspend fun pullClipboard(node: String) {
        val myId = me?.nodeId ?: ""
        call { it.send(node, ClipboardSync.request(myId)) }
    }

    suspend fun shareNotification(title: String, body: String) {
        call { it.shareNotification(title, body, "my-domain") }
        addActivity(str(R.string.act_notification_sent, title), Tone.Ok)
    }

    suspend fun requestApps(node: String) = call { it.requestApps(node) }

    // Quick device actions: each returns the transport used, or "queued"
    // (delivered when the device is reachable; a sleeping phone is woken).

    /** Ask a device for its battery/charging (reply: `device_status`). */
    suspend fun requestStatus(node: String): String = call { it.requestStatus(node) }

    /** Ring a device (find my device). */
    suspend fun ring(node: String): String = call { it.ring(node) }

    /** Control what plays on [node] (core `media_command`: "play", "pause",
     *  "next", "previous", "seek" with [value] ms, "volume_up",
     *  "volume_down", "volume_set" with [value], "session"). A play, pause,
     *  seek or volume shows at once; the device's next now-playing confirms. */
    suspend fun mediaCommand(node: String, action: String, value: Long? = null, session: String? = null) {
        call { it.mediaCommand(node, action, value, session) }
        val cur = nowPlaying[node] ?: return
        val now = System.currentTimeMillis()
        nowPlaying[node] = when (action) {
            "play", "pause" -> cur.copy(playing = action == "play", position = cur.positionNow(now), at = now)
            "seek" -> if (value != null) cur.copy(position = value, at = now) else cur
            "volume_set" -> cur.copy(volume = value?.toInt())
            else -> cur
        }
    }

    /** Keep hearing what plays on [node] for a while (a screen showing it
     *  calls this every few minutes). */
    suspend fun mediaWatch(node: String) {
        runCatching { call { it.mediaWatch(node) } }
    }

    /** Run [name] on [node], then ask for its status again so the buttons
     *  show the new state. */
    suspend fun runAction(node: String, name: String, args: JSONObject = JSONObject()): String {
        val via = call { it.runAction(node, name, args.toString()) }
        delay(400)
        runCatching { requestStatus(node) }
        return via
    }

    /** Presentation remote: [key] "next", "previous" or "black" on a
     *  computer that lists `slides` in its actions. Not followed by a status
     *  request (nothing to show changes). */
    suspend fun slide(node: String, key: String): String =
        call { it.runAction(node, "slides", JSONObject().put("key", key).toString()) }

    /** Open a web link on a device. */
    suspend fun openUrl(node: String, url: String): String = call { it.openUrl(node, url) }

    /** Open a web link on a device in app [pkg] (e.g. "com.google.android.youtube");
     *  a device without it uses its browser. */
    suspend fun openUrlIn(node: String, url: String, pkg: String): String = call { it.openUrlIn(node, url, pkg) }

    /** Search for [query] in [app] ("youtube", "spotify" or "web") on a
     *  device that lists `search` (its status' `search_apps` say which). */
    suspend fun search(node: String, app: String, query: String): String = call { it.searchInApp(node, app, query) }

    /**
     * Send a picked or shared file ([uri]) to a device: copied to the cache
     * first (the core sends from a path), then handed to the core. Returns
     * the transfer id; progress and the outcome arrive as events.
     */
    suspend fun sendFile(node: String, uri: Uri): String {
        val file = withContext(Dispatchers.IO) { OutgoingFiles.copy(app, uri) }
        val id = try {
            call { it.sendFile(node, file.path) }
        } catch (e: Exception) {
            OutgoingFiles.discard(file)
            throw e
        }
        OutgoingFiles.track(id, file)
        val size = file.length()
        // Kept with the transfer, for "Send again".
        TransferLog.sending(id, node, file.name, uri.toString(), size)
        onMain {
            updateTransfer(id, { Transfer(it, node, file.name, false) }) {
                if (it.total == 0L) it.copy(total = size) else it
            }
        }
        return id
    }

    /** Process-wide work that must outlive a screen (copying a big file
     *  for sending keeps going when the user navigates away). */
    private val appScope = CoroutineScope(SupervisorJob() + Dispatchers.Main)

    /** Send [uris] to [node], off the screen's lifetime: one on its own,
     *  several as one batch (one row in Transfers, the files inside).
     *  [onError] gets the first failure. */
    fun sendFiles(node: String, uris: List<Uri>, onDone: () -> Unit = {}, onError: (Throwable) -> Unit = {}) {
        appScope.launch {
            try {
                if (uris.size == 1) sendFile(node, uris[0]) else sendBatch(node, uris)
            } catch (e: Exception) {
                onError(e)
            }
            onDone()
        }
    }

    /** Send several picked files as one batch: copied to the cache first
     *  (into one folder, which goes once nothing is being sent from it,
     *  like a folder's copy), then handed to the core. How many go. */
    suspend fun sendBatch(node: String, uris: List<Uri>): Int {
        val files = withContext(Dispatchers.IO) { OutgoingFolders.stageFiles(app, uris) }
        val dir = files.firstOrNull()?.parentFile ?: throw java.io.IOException(str(R.string.batch_nothing))
        val n = try {
            call { it.sendBatch(node, files.map { f -> f.path }) }
        } catch (e: Exception) {
            OutgoingFolders.discard(dir)
            throw e
        }
        OutgoingFolders.track(dir)
        return n.toInt()
    }

    /** Pause ([paused]) or resume a transfer or a batch, either way. */
    suspend fun pauseFile(id: String, paused: Boolean) {
        call { it.pauseFile(id, paused) }
        onMain { TransferLog.paused(id, paused) }
    }

    /** Send a failed transfer again; the id to follow. */
    suspend fun retryFile(id: String): String = call { it.retryFile(id) }

    /** [pauseFile] / [retryFile] from a click handler; [onError] gets why
     *  it couldn't (on the main thread). */
    fun pauseFileAsync(id: String, paused: Boolean, onError: (Throwable) -> Unit = {}) {
        appScope.launch { try { pauseFile(id, paused) } catch (e: Exception) { onError(e) } }
    }

    fun retryFileAsync(id: String, onDone: () -> Unit = {}, onError: (Throwable) -> Unit = {}) {
        appScope.launch {
            try { retryFile(id); onDone() } catch (e: Exception) { onError(e) }
        }
    }

    /**
     * Send a folder picked with the system's folder picker ([tree], a content
     * tree Uri) to a device: copied to the cache first (the core sends from a
     * path; [onProgress] gets the files copied so far and how many, on the
     * main thread), then handed to the core, which sends each file as its own
     * transfer. Returns how many files go; the copy is deleted once they're
     * done (see [OutgoingFolders]).
     */
    suspend fun sendFolder(node: String, tree: Uri, onProgress: (Int, Int) -> Unit = { _, _ -> }): Int {
        val dir = withContext(Dispatchers.IO) {
            OutgoingFolders.stage(app, tree) { done, total -> onMain { onProgress(done, total) } }
        }
        val n = try {
            call { it.sendFolder(node, dir.path) }
        } catch (e: Exception) {
            OutgoingFolders.discard(dir)
            throw e
        }
        OutgoingFolders.track(dir)
        return n.toInt()
    }

    /** [sendFolder] off the screen's lifetime (copying a big folder keeps
     *  going when the user navigates away). [onSent] gets how many files go. */
    fun sendFolderAsync(
        node: String,
        tree: Uri,
        onProgress: (Int, Int) -> Unit = { _, _ -> },
        onSent: (Int) -> Unit = {},
        onError: (Throwable) -> Unit = {},
        onDone: () -> Unit = {},
    ) {
        appScope.launch {
            try {
                onSent(sendFolder(node, tree, onProgress))
            } catch (e: Exception) {
                onError(e)
            } finally {
                onDone()
            }
        }
    }

    /** Send [text] to a device: onto its clipboard ([mode] "clipboard") or
     *  shown there as a notification ("notify"). */
    suspend fun sendText(node: String, text: String, mode: String = "clipboard", sensitive: Boolean = false): String {
        // A [sensitive] one (a password) is marked so: the device keeps it
        // out of its history and notices ([ClipPrivacy]); not kept here either.
        val how = call { if (sensitive) it.sendTextMarked(node, text, mode, true) else it.sendText(node, text, mode) }
        // Sent (or queued): kept in the clipboard history.
        if (mode == "clipboard" && !sensitive) ClipLog.sent(node, text)
        return how
    }

    /** Text from the share sheet: a lone web link opens on the device, any
     *  other text goes onto its clipboard. */
    suspend fun sendSharedText(node: String, text: String): String =
        if (isSharedLink(text)) openUrl(node, text.trim()) else sendText(node, text, "clipboard")

    /** Is shared [text] a lone web link ([sendSharedText] opens it)? */
    fun isSharedLink(text: String): Boolean {
        val t = text.trim()
        val scheme = runCatching { Uri.parse(t).scheme?.lowercase() }.getOrNull()
        return (scheme == "http" || scheme == "https") && t.none(Char::isWhitespace)
    }

    /** [sendSharedText] from a click handler, off the screen's lifetime. */
    fun sendSharedTextAsync(node: String, text: String, onError: (Throwable) -> Unit = {}) {
        appScope.launch {
            try { sendSharedText(node, text) } catch (e: Exception) { onError(e) }
        }
    }

    /** Ask [node] what's in [path] of its shared folder [root] ("" lists its
     *  shared folders). Returns the request id; the answer lands in
     *  [listings] under it. */
    suspend fun listFolder(node: String, root: String = "", path: String = "", offset: Long = 0): String =
        call { it.listFolder(node, root, path, offset.toULong()) }

    /** Fetch a file from [node]'s shared folder; it shows up in [transfers]
     *  and is saved to Downloads/my-domain like any received file. */
    suspend fun downloadFile(node: String, root: String, path: String): String =
        call { it.downloadFile(node, root, path) }

    /** Stop a transfer either way (a `file_failed` "cancelled" follows). */
    suspend fun cancelFile(id: String) = call { it.cancelFile(id) }

    /** [cancelFile] from a click handler (no coroutine there). */
    fun cancelFileAsync(id: String) {
        appScope.launch { runCatching { cancelFile(id) } }
    }

    // --- gallery, photo backup, battery alerts, keep awake ----------------------

    /** Ask [node] for a page of its photos and videos (newest first); the
     *  answer is a `gallery_listing` event with the returned request id,
     *  then `gallery_thumb`s. (No screen here yet: the computer shows the
     *  phone's gallery.) */
    suspend fun listGallery(node: String, offset: Long = 0, limit: Int = 60, thumbs: Boolean = true): String =
        call { it.listGallery(node, offset.toULong(), limit.toUInt(), thumbs) }

    /** Fetch a listed photo or video from [node]; it arrives like any file. */
    suspend fun downloadGalleryItem(node: String, id: String, name: String): String =
        call { it.downloadGalleryItem(node, id, name) }

    // --- synced folders (SyncFolders.kt) ---

    /** Read the synced folders now (also kept current from events). */
    fun refreshSync() {
        val list = runCatching { engine()?.syncPairs() }.getOrNull() ?: return
        onMain {
            syncPairs.clear()
            syncPairs.addAll(list)
        }
    }

    /** Accept invitation [id], keeping the folder at [path] on this phone. */
    suspend fun syncAccept(id: String, path: String) {
        call { it.syncAccept(id, path) }
        refreshSync()
    }

    /** Stop syncing on both devices (or decline an invitation); files stay. */
    suspend fun syncRemove(id: String) {
        call { it.syncRemove(id) }
        refreshSync()
    }

    suspend fun syncPause(id: String, paused: Boolean) {
        call { it.syncPause(id, paused) }
        refreshSync()
    }

    suspend fun syncNow(id: String) = call { it.syncNow(id) }

    /** Read the backup state now (also kept current from events). */
    fun refreshBackup() {
        val s = runCatching { engine()?.backupState() }.getOrNull() ?: return
        onMain { backup = s }
    }

    /** Turn the photo backup on or off, choose the computer ([target], a
     *  node id) and whether to wait for the charger. */
    suspend fun setBackup(enabled: Boolean, target: String, chargingOnly: Boolean) {
        withContext(Dispatchers.IO) { com.mydomain.android.PhotoBackup.configure(enabled, target, chargingOnly) }
        refreshBackup()
    }

    /** Whether this phone tells your other devices when its battery is low
     *  ([low] percent) or charged ([charged] percent, 0 = when full). */
    fun batteryAlerts(): com.mydomain.android.ffi.BatteryAlertConfig? = runCatching { engine()?.batteryAlerts() }.getOrNull()

    suspend fun setBatteryAlerts(enabled: Boolean, low: Int = 15, charged: Int = 0) =
        call { it.setBatteryAlerts(enabled, low.coerceIn(1, 99).toUByte(), charged.coerceIn(0, 100).toUByte()) }

    /** Keep [node] awake for [minutes] (0 = stop), if it lists "keep_awake". */
    suspend fun keepAwake(node: String, minutes: Int): String =
        call { it.keepAwake(node, minutes.coerceAtLeast(0).toUInt()) }

    // --- touchpad and keyboard for a computer -----------------------------------

    /** Move [node]'s pointer by [dx], [dy] pixels. Not a suspend call: the
     *  core only adds it to what's pending and sends that about 50 times a
     *  second, so call it for every touch move. Returns the error, if any
     *  (no link yet: the core starts connecting). */
    fun inputMove(node: String, dx: Int, dy: Int): Throwable? =
        runCatching { engine()?.inputMove(node, dx, dy) }.exceptionOrNull()

    /** Scroll [node] by [dx], [dy] pixels (positive [dy]: down). Batched like moves. */
    fun inputScroll(node: String, dx: Int, dy: Int): Throwable? =
        runCatching { engine()?.inputScroll(node, dx, dy) }.exceptionOrNull()

    /** Click "left", "right" or "middle" on [node]. */
    suspend fun inputClick(node: String, button: String) = call { it.inputClick(node, button) }

    /** Type [text] on [node]. */
    suspend fun inputText(node: String, text: String) = call { it.inputText(node, text) }

    /** Press "enter", "backspace", "left", "right", "up", "down" or "esc" on [node]. */
    suspend fun inputKey(node: String, key: String) = call { it.inputKey(node, key) }

    /**
     * Send the picture on this phone's clipboard to [node]'s clipboard (as a
     * PNG). False when the clipboard holds no picture. Reading the clipboard
     * needs the app on screen (Android 10+), which a button press is.
     */
    suspend fun sendClipboardImage(ctx: Context, node: String): Boolean {
        val cm = ctx.getSystemService(android.content.ClipboardManager::class.java) ?: return false
        val clip = cm.primaryClip ?: return false
        val uri = (0 until clip.itemCount).firstNotNullOfOrNull { clip.getItemAt(it).uri } ?: return false
        val type = ctx.contentResolver.getType(uri).orEmpty()
        if (!type.startsWith("image/")) return false
        val file = withContext(Dispatchers.IO) {
            val bmp = ctx.contentResolver.openInputStream(uri)?.use { android.graphics.BitmapFactory.decodeStream(it) }
                ?: return@withContext null
            val dir = java.io.File(ctx.cacheDir, "clipboard").apply { mkdirs() }
            java.io.File(dir, "Copied picture ${System.currentTimeMillis() / 1000}.png").also { f ->
                f.outputStream().use { bmp.compress(android.graphics.Bitmap.CompressFormat.PNG, 100, it) }
            }
        } ?: return false
        call { it.sendClipboardImage(node, file.absolutePath) }
        ClipLog.pictureSent(ctx, node, file)
        return true
    }

    /** Send kept picture [p] (clipboard history) to [node]'s clipboard
     *  again, from a copy (so the kept one may go meanwhile). */
    suspend fun sendKeptPicture(ctx: Context, node: String, p: ClipPicture) {
        val kept = ClipLog.pictureFile(p) ?: throw java.io.FileNotFoundException(p.file)
        val file = withContext(Dispatchers.IO) {
            val dir = java.io.File(ctx.cacheDir, "clipboard").apply { mkdirs() }
            kept.copyTo(java.io.File(dir, "Copied picture ${System.currentTimeMillis() / 1000}.png"), overwrite = true)
        }
        call { it.sendClipboardImage(node, file.absolutePath) }
        ClipLog.pictureSent(ctx, node, file)
    }

    /** Put kept picture [p] on this phone's clipboard: a content Uri through
     *  the app's FileProvider, so the app that pastes may read it. */
    fun copyKeptPicture(ctx: Context, p: ClipPicture) {
        val file = ClipLog.pictureFile(p)?.takeIf { it.isFile } ?: throw java.io.FileNotFoundException(p.file)
        val uri = androidx.core.content.FileProvider.getUriForFile(ctx, "${ctx.packageName}.files", file)
        val cm = ctx.getSystemService(android.content.ClipboardManager::class.java) ?: return
        cm.setPrimaryClip(android.content.ClipData.newUri(ctx.contentResolver, "Picture", uri))
    }

    /** Save kept picture [p] to Pictures/my-domain (MediaStore, Android 10+).
     *  False on older Android, which would need the storage permission. */
    suspend fun saveKeptPicture(ctx: Context, p: ClipPicture): Boolean {
        if (android.os.Build.VERSION.SDK_INT < android.os.Build.VERSION_CODES.Q) return false
        val file = ClipLog.pictureFile(p)?.takeIf { it.isFile } ?: throw java.io.FileNotFoundException(p.file)
        withContext(Dispatchers.IO) {
            val cr = ctx.contentResolver
            val values = android.content.ContentValues().apply {
                put(android.provider.MediaStore.Images.Media.DISPLAY_NAME, "Picture ${System.currentTimeMillis() / 1000}.png")
                put(android.provider.MediaStore.Images.Media.MIME_TYPE, "image/png")
                put(android.provider.MediaStore.Images.Media.RELATIVE_PATH, "${android.os.Environment.DIRECTORY_PICTURES}/my-domain")
                put(android.provider.MediaStore.Images.Media.IS_PENDING, 1)
            }
            val uri = cr.insert(android.provider.MediaStore.Images.Media.EXTERNAL_CONTENT_URI, values)
                ?: throw java.io.IOException("MediaStore insert failed")
            try {
                cr.openOutputStream(uri)?.use { out -> file.inputStream().use { it.copyTo(out) } }
                    ?: throw java.io.IOException("can't write the picture")
                cr.update(uri, android.content.ContentValues().apply { put(android.provider.MediaStore.Images.Media.IS_PENDING, 0) }, null, null)
            } catch (e: Exception) {
                cr.delete(uri, null, null)
                throw e
            }
        }
        return true
    }

    /** Whether [node] said it may be woken from sleep (Wake-on-LAN). */
    fun canWake(node: String): Boolean = runCatching { engine()?.canWake(node) }.getOrNull() == true

    /** Wake [node] from sleep: its magic packet, broadcast on this network,
     *  and sent by your other devices linked now on its network ([WakeResult.relays]). */
    suspend fun wake(node: String): com.mydomain.android.ffi.WakeResult =
        call { it.wakeDevice(node) }.also { ActivityLog.record("wake", node) }

    /** Start ([on]) or stop watching [node]'s screen; null when it went out. */
    fun screenWatch(node: String, on: Boolean): Throwable? =
        runCatching { engine()?.screenWatch(node, on) }.exceptionOrNull()

    /** Put [node]'s pointer at [x], [y] (0 to 10000 across its screen). */
    suspend fun inputPoint(node: String, x: Int, y: Int) =
        call { it.inputPoint(node, x.toUShort(), y.toUShort()) }

    /** A [fingers]-finger swipe (3 or 4) on [node]; [dir] is "left", "right", "up" or "down". */
    suspend fun inputGesture(node: String, fingers: Int, dir: String) =
        call { it.inputGesture(node, fingers.toUByte(), dir) }

    suspend fun toggleApp(node: String, pkg: String) {
        val cur = apps[node] ?: return
        val next = if (pkg in cur.subscribed) cur.subscribed - pkg else cur.subscribed + pkg
        call { it.subscribeApps(node, JSONArray(next.toList()).toString()) }
        onMain { apps[node] = cur.copy(subscribed = next) }
    }

    suspend fun signOut() {
        call { it.logout() }
        onMain {
            messages.clear(); activity.clear(); apps.clear(); progress.clear(); links.clear(); deviceStatus.clear(); clipHistory.clear(); listings.clear(); nowPlaying.clear()
        }
        reload()
    }
}

/** The clipboard control messages NetService exchanges over chat links. */
object ClipboardSync {
    private val TYPES = setOf("clipboard", "clipboard_request", "clipboard_response")

    fun isControl(text: String): Boolean =
        text.startsWith("{") && runCatching { JSONObject(text).optString("type") in TYPES }.getOrDefault(false)

    fun request(from: String): String =
        JSONObject().put("type", "clipboard_request").put("from", from).toString()
}

/** A `nearby-media` event (or `nearbyStatus`) as the screens use it. */
fun nearbyState(o: JSONObject): NearbyBadge.State = NearbyBadge.State(
    media = o.optBoolean("media"),
    near = if (o.isNull("near")) null else o.optString("near").ifEmpty { null },
    lowLatency = o.optBoolean("low_latency"),
    path = o.optString("path"),
    latencyMs = if (o.isNull("latency_ms")) null else o.optInt("latency_ms"),
    beforeMs = if (o.isNull("before_ms")) null else o.optInt("before_ms"),
    rttMs = if (o.isNull("rtt_ms")) null else o.optInt("rtt_ms"),
)
