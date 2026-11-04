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
import com.mydomain.android.ffi.DeviceIdentity
import com.mydomain.android.ffi.MdEngine
import com.mydomain.android.ffi.PeerInfo
import com.mydomain.android.ffi.Session
import com.mydomain.android.ffi.runningEngine
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import org.json.JSONArray
import org.json.JSONObject

/**
 * One chat bubble. Outgoing messages carry [msgId] (sent to the core) and stay
 * [pending] until the peer acknowledges it (`message_delivered` with that id).
 * [queued]: there was no link when it was sent, so the core holds it until
 * the device is connected.
 */
data class ChatMessage(
    val id: Long, val node: String, val incoming: Boolean, val text: String, val ts: Long,
    val msgId: String? = null,
    val queued: Boolean = false,
    val delivered: Boolean = false,
) {
    val pending: Boolean get() = !incoming && !delivered
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

/** What's playing on a device (`now_playing`, also in its status). */
data class Playing(val title: String, val artist: String?, val app: String?, val playing: Boolean)

private fun playingOf(o: JSONObject?): Playing? {
    val title = o?.optString("title").orEmpty()
    if (o == null || title.isEmpty()) return null
    return Playing(
        title,
        o.optString("artist").ifEmpty { null },
        o.optString("app").ifEmpty { null },
        o.optString("state") == "playing",
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

    /** Text or a link shared to the app from another app, waiting for the
     *  user to pick a device (files win if both came). */
    var pendingText by mutableStateOf<String?>(null); private set

    /** Answers to [listFolder], by request id (this process only). */
    val listings = mutableStateMapOf<String, FolderListing>()

    /** Clipboard auto-sync with the other devices (NetService acts on it). */
    var clipSync by mutableStateOf(false); private set

    /** Where this phone's photo backup stands (`backup_state` events). */
    var backup by mutableStateOf<com.mydomain.android.ffi.BackupInfo?>(null); private set

    /** Recent clipboard values, newest first (memory only, never saved). */
    val clipHistory = mutableStateListOf<ClipEntry>()

    /** Text the app itself just put on the system clipboard from
     *  [clipHistory]; the clipboard listener skips recording it once. */
    @Volatile private var recopied: String? = null

    fun engine(): MdEngine? = runningEngine()

    /** Events that change what the home-screen widget shows. */
    private val WIDGET_KINDS = setOf("peers_updated", "peer_connected", "peer_disconnected", "device_status")

    /** The app context, for strings built outside the screens (set by
     *  MainActivity and NetService, whichever starts first). */
    lateinit var app: Context
        private set

    fun init(context: Context) {
        app = context.applicationContext
        BatteryLog.load(app)
        ClipLog.load(app)
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
        Thread { engine()?.setBackground(!now) }.start()
    }

    fun peerName(node: String?): String =
        peers.firstOrNull { it.nodeId == node }?.name?.let(::niceName) ?: str(R.string.a_device)

    private fun onMain(block: () -> Unit) {
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
        }
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
        // A code event's payload holds the code itself: never log it.
        labLog(if (name.startsWith("code")) name else "$name ${payloadJson.take(200)}")
        val o = runCatching { JSONObject(payloadJson) }.getOrNull() ?: return
        val kind = o.optString("kind")
        val peersNow = if (kind == "peers_updated") engine()?.peers() else null
        if (kind == "backup_state") engine()?.let { e -> runCatching { e.backupState() }.getOrNull()?.let { s -> onMain { backup = s } } }
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
        }
    }

    private fun apply(o: JSONObject, peersNow: List<PeerInfo>?) {
        val node = o.optString("node_id")
        when (o.optString("kind")) {
            "peers_updated" -> peersNow?.let { peers = it }
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
            "message_delivered" -> {
                val id = o.optString("id")
                messages.replaceAll { if (!it.incoming && it.msgId == id) it.copy(delivered = true) else it }
            }
            "message_received" -> {
                if (!o.optBoolean("ok", true)) {
                    addActivity(str(R.string.act_unreadable), Tone.Error)
                    return
                }
                val text = o.optString("text")
                if (ClipboardSync.isControl(text)) return
                val from = node.ifEmpty { peers.firstOrNull { it.ip == o.optString("ip") }?.nodeId ?: "" }
                messages.add(ChatMessage(++seq, from, true, text, System.currentTimeMillis()))
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
                if (o.optString("mode") == "clipboard") ClipLog.received(o.optString("from"), o.optString("text"))
                addActivity(
                    str(if (o.optString("mode") == "clipboard") R.string.act_text_copied else R.string.act_text_from, peerName(o.optString("from")), o.optString("text").take(200)),
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
            "file_progress" -> updateTransfer(o.optString("id"), { Transfer(it, node, o.optString("name"), o.optBoolean("incoming")) }) {
                if (it.state != TransferState.Active) it
                else it.copy(done = o.optLong("done"), total = o.optLong("total"))
            }
            "file_received" -> {
                val from = o.optString("from")
                val name = o.optString("name")
                updateTransfer(o.optString("id"), { Transfer(it, from, name, true) }) {
                    it.copy(state = TransferState.Done, done = o.optLong("size"), total = o.optLong("size"))
                }
                addActivity(str(R.string.act_file_saved, name, peerName(from)), Tone.Ok)
            }
            "file_sent" -> {
                val name = o.optString("name")
                updateTransfer(o.optString("id"), { Transfer(it, node, name, false) }) {
                    it.copy(state = TransferState.Done, done = it.total)
                }
                addActivity(str(R.string.act_file_sent, name, peerName(node)), Tone.Ok)
            }
            "file_failed" -> {
                val name = o.optString("name")
                val incoming = o.optBoolean("incoming")
                val reason = o.optString("reason")
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
        val via = try {
            call { it.sendWithId(node, text, msgId) }
        } catch (e: Exception) {
            onMain { messages.removeAll { it.msgId == msgId } }
            throw e
        }
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

    /** Forget the chat with [node], on this phone only. */
    fun clearChat(node: String) {
        messages.removeAll { it.node == node }
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

    /** Send [uris] to [node] one after another, off the screen's lifetime.
     *  [onError] gets the first failure (each file is tried). */
    fun sendFiles(node: String, uris: List<Uri>, onDone: () -> Unit = {}, onError: (Throwable) -> Unit = {}) {
        appScope.launch {
            var failed: Throwable? = null
            for (u in uris) {
                try { sendFile(node, u) } catch (e: Exception) { if (failed == null) failed = e }
            }
            failed?.let(onError)
            onDone()
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
    suspend fun sendText(node: String, text: String, mode: String = "clipboard"): String {
        val how = call { it.sendText(node, text, mode) }
        // Sent (or queued): kept in the clipboard history.
        if (mode == "clipboard") ClipLog.sent(node, text)
        return how
    }

    /** Text from the share sheet: a lone web link opens on the device, any
     *  other text goes onto its clipboard. */
    suspend fun sendSharedText(node: String, text: String): String {
        val t = text.trim()
        val scheme = runCatching { Uri.parse(t).scheme?.lowercase() }.getOrNull()
        val link = (scheme == "http" || scheme == "https") && t.none(Char::isWhitespace)
        return if (link) openUrl(node, t) else sendText(node, text, "clipboard")
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
        return true
    }

    /** Whether [node] ever said how to wake it from sleep (Wake-on-LAN). */
    fun canWake(node: String): Boolean = runCatching { engine()?.canWake(node) }.getOrNull() == true

    /** Wake [node] from sleep: its magic packet, broadcast on this network. */
    suspend fun wake(node: String) = call { it.wakeDevice(node) }

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
