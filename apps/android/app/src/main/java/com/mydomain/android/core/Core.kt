package com.mydomain.android.core

import android.os.Handler
import android.os.Looper
import android.util.Log
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateListOf
import androidx.compose.runtime.mutableStateMapOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import com.mydomain.android.BuildConfig
import com.mydomain.android.ffi.DeviceIdentity
import com.mydomain.android.ffi.MdEngine
import com.mydomain.android.ffi.PeerInfo
import com.mydomain.android.ffi.Session
import com.mydomain.android.ffi.runningEngine
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import org.json.JSONArray
import org.json.JSONObject

data class ChatMessage(val id: Long, val node: String, val incoming: Boolean, val text: String, val ts: Long)

enum class Tone { Info, Ok, Warn, Error }

data class ActivityItem(val id: Long, val ts: Long, val text: String, val tone: Tone)

data class AppEntry(val pkg: String, val label: String)

data class AppsState(val apps: List<AppEntry>, val subscribed: Set<String>)

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
    val messages = mutableStateListOf<ChatMessage>()
    val activity = mutableStateListOf<ActivityItem>()
    val apps = mutableStateMapOf<String, AppsState>()
    val devLog = mutableStateListOf<DevEvent>()

    /** Clipboard auto-sync with the other devices (NetService acts on it). */
    var clipSync by mutableStateOf(false); private set

    fun engine(): MdEngine? = runningEngine()

    fun peerName(node: String?): String =
        peers.firstOrNull { it.nodeId == node }?.name?.let(::niceName) ?: "a device"

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

    fun enableClipSync(on: Boolean) = onMain { clipSync = on }

    // --- events --------------------------------------------------------------

    /** Every core event, from NetService's listener (core threads). */
    fun onEvent(name: String, payloadJson: String) {
        if (BuildConfig.DEBUG) Log.i("mydomain_event", "$name ${payloadJson.take(200)}")
        val o = runCatching { JSONObject(payloadJson) }.getOrNull() ?: return
        val peersNow = if (o.optString("kind") == "peers_updated") engine()?.peers() else null
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
                if (!was) addActivity("Connected to ${peerName(node)}", Tone.Ok)
            }
            "peer_disconnected" -> {
                links.remove(node)
                progress.remove(node)
                addActivity("${peerName(node)} disconnected")
            }
            "connect_progress" -> progress[node] = o.optString("stage")
            "message_received" -> {
                if (!o.optBoolean("ok", true)) {
                    addActivity("A message couldn't be read: that device uses a different key", Tone.Error)
                    return
                }
                val text = o.optString("text")
                if (ClipboardSync.isControl(text)) return
                val from = node.ifEmpty { peers.firstOrNull { it.ip == o.optString("ip") }?.nodeId ?: "" }
                messages.add(ChatMessage(++seq, from, true, text, System.currentTimeMillis()))
                addActivity("${peerName(from)}: $text")
            }
            "notification" -> {
                val body = o.optString("body")
                addActivity(o.optString("title") + if (body.isNotEmpty()) " — $body" else "")
            }
            "call_notification" -> addActivity("Call from ${o.optString("caller")} (${o.optString("state")})")
            "call_history" -> addActivity("Call history received")
            "clipboard" -> addActivity("Clipboard synced")
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

    /** Send a chat message; returns the transport used. */
    suspend fun sendMessage(node: String, text: String): String {
        val via = call { it.send(node, text) }
        onMain { messages.add(ChatMessage(++seq, node, false, text, System.currentTimeMillis())) }
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
        addActivity("Sent “$title” to your devices", Tone.Ok)
    }

    suspend fun requestApps(node: String) = call { it.requestApps(node) }

    suspend fun toggleApp(node: String, pkg: String) {
        val cur = apps[node] ?: return
        val next = if (pkg in cur.subscribed) cur.subscribed - pkg else cur.subscribed + pkg
        call { it.subscribeApps(node, JSONArray(next.toList()).toString()) }
        onMain { apps[node] = cur.copy(subscribed = next) }
    }

    suspend fun signOut() {
        call { it.logout() }
        onMain {
            messages.clear(); activity.clear(); apps.clear(); progress.clear(); links.clear()
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
