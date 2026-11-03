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
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import com.mydomain.android.BuildConfig
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
    val storageFree: Long? = null, val storageTotal: Long? = null,
)

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

    /** Text or a link shared to the app from another app, waiting for the
     *  user to pick a device (files win if both came). */
    var pendingText by mutableStateOf<String?>(null); private set

    /** Answers to [listFolder], by request id (this process only). */
    val listings = mutableStateMapOf<String, FolderListing>()

    /** Clipboard auto-sync with the other devices (NetService acts on it). */
    var clipSync by mutableStateOf(false); private set

    /** Recent clipboard values, newest first (memory only, never saved). */
    val clipHistory = mutableStateListOf<ClipEntry>()

    /** Text the app itself just put on the system clipboard from
     *  [clipHistory]; the clipboard listener skips recording it once. */
    @Volatile private var recopied: String? = null

    fun engine(): MdEngine? = runningEngine()

    /** The app context, for strings built outside the screens (set by
     *  MainActivity and NetService, whichever starts first). */
    lateinit var app: Context
        private set

    fun init(context: Context) {
        app = context.applicationContext
    }

    fun str(@StringRes id: Int, vararg args: Any): String = app.getString(id, *args)

    fun plural(@PluralsRes id: Int, n: Int, vararg args: Any): String =
        app.resources.getQuantityString(id, n, *args)

    /** Is the app on screen? Off screen, the engine closes idle links
     *  (battery); a send re-opens them and pushes still arrive. */
    @Volatile var visible = false
        private set

    fun setVisible(on: Boolean) {
        visible = on
        labLog(if (on) "app visible" else "app in background")
        Thread { engine()?.setBackground(!on) }.start()
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
        // A code event's payload holds the code itself: never log it.
        labLog(if (name.startsWith("code")) name else "$name ${payloadJson.take(200)}")
        val o = runCatching { JSONObject(payloadJson) }.getOrNull() ?: return
        val kind = o.optString("kind")
        val peersNow = if (kind == "peers_updated") engine()?.peers() else null
        // An outgoing transfer ended: its cache copy can go.
        if (kind == "file_sent" || (kind == "file_failed" && !o.optBoolean("incoming"))) OutgoingFiles.end(o.optString("id"))
        onMain {
            devLog.add(0, DevEvent(System.currentTimeMillis(), name, payloadJson))
            if (devLog.size > 500) devLog.removeRange(500, devLog.size)
            apply(o, peersNow)
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
                    ramFree = st.optJSONObject("ram")?.optLong("free"),
                    ramTotal = st.optJSONObject("ram")?.optLong("total"),
                    storageFree = st.optJSONObject("storage")?.optLong("free"),
                    storageTotal = st.optJSONObject("storage")?.optLong("total"),
                )
            }
            "ring" -> addActivity(str(R.string.act_ring, peerName(o.optString("from"))), Tone.Warn)
            "open_url" -> addActivity(str(R.string.act_link, peerName(o.optString("from")), o.optString("url")))
            "clipboard" -> addActivity(str(R.string.act_clipboard_synced))
            "text_shared" -> addActivity(
                str(if (o.optString("mode") == "clipboard") R.string.act_text_copied else R.string.act_text_from, peerName(o.optString("from")), o.optString("text").take(200)),
            )
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

    /** Send [text] to a device: onto its clipboard ([mode] "clipboard") or
     *  shown there as a notification ("notify"). */
    suspend fun sendText(node: String, text: String, mode: String = "clipboard"): String =
        call { it.sendText(node, text, mode) }

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
