package com.mydomain.android

import android.Manifest
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.net.ConnectivityManager
import android.net.Network
import android.net.NetworkCapabilities
import android.net.NetworkRequest
import android.net.wifi.WifiManager
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.util.Log
import androidx.annotation.StringRes
import androidx.compose.runtime.mutableStateListOf
import androidx.core.app.NotificationCompat
import com.mydomain.android.core.Automation
import com.mydomain.android.core.AutomationBook
import com.mydomain.android.core.Core
import com.mydomain.android.core.Edge
import com.mydomain.android.core.Trigger
import com.mydomain.android.core.TriggerKind
import com.mydomain.android.core.friendlyError
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.launch
import org.json.JSONObject
import java.io.File
import java.util.concurrent.Executors
import java.util.concurrent.atomic.AtomicInteger

/** Something an automation can do: [action] with [args] (JSON object
 *  text). [always]: offered whatever the device lists in its status. */
data class AutoPreset(
    val id: String,
    val action: String,
    val args: String,
    @StringRes val label: Int,
    val always: Boolean = false,
)

/** The presets, as the desktop's `lib/presets.ts`. */
object AutoPresets {
    /** Sends a note, shown on the device as a notification. */
    const val SEND_TEXT = "send_text"

    /** On this phone. */
    val PHONE = listOf(
        AutoPreset("notify", AutomationBook.NOTIFY, "{}", R.string.automation_preset_notify, always = true),
        AutoPreset("ringer_normal", "ringer", """{"mode":"normal"}""", R.string.automation_preset_ringer_normal),
        AutoPreset("ringer_vibrate", "ringer", """{"mode":"vibrate"}""", R.string.automation_preset_ringer_vibrate),
        AutoPreset("ringer_silent", "ringer", """{"mode":"silent"}""", R.string.automation_preset_ringer_silent),
        AutoPreset("torch_off", "flashlight", """{"on":false}""", R.string.automation_preset_torch_off),
        AutoPreset("awake_off", "keep_awake", """{"minutes":0}""", R.string.automation_preset_awake_off),
        AutoPreset("media", "media", """{"key":"play_pause"}""", R.string.automation_preset_media),
    )

    /** On another device (a computer, mostly). */
    val DEVICE = listOf(
        AutoPreset("lock", "lock", "{}", R.string.automation_preset_lock),
        AutoPreset("ring", "ring", "{}", R.string.automation_preset_ring, always = true),
        AutoPreset("note", SEND_TEXT, "{}", R.string.automation_preset_note, always = true),
        AutoPreset("mute", "mute", """{"on":true}""", R.string.automation_preset_mute),
        AutoPreset("unmute", "mute", """{"on":false}""", R.string.automation_preset_unmute),
        AutoPreset("media", "media", """{"key":"play_pause"}""", R.string.automation_preset_media),
        AutoPreset("awake_off", "keep_awake", """{"minutes":0}""", R.string.automation_preset_awake_off),
        AutoPreset("ringer_normal", "ringer", """{"mode":"normal"}""", R.string.automation_preset_ringer_normal),
        AutoPreset("ringer_vibrate", "ringer", """{"mode":"vibrate"}""", R.string.automation_preset_ringer_vibrate),
        AutoPreset("ringer_silent", "ringer", """{"mode":"silent"}""", R.string.automation_preset_ringer_silent),
        AutoPreset("torch_off", "flashlight", """{"on":false}""", R.string.automation_preset_torch_off),
    )

    /** What this phone can do to itself (Controls.actions). */
    fun phoneActions(): Set<String> {
        val arr = Controls.actions()
        return (0 until arr.length()).map { arr.optString(it) }.toSet()
    }

    /** The presets for [node] ("" = this phone) that it can do. */
    fun offered(node: String): List<AutoPreset> = if (node.isEmpty()) {
        val acts = phoneActions()
        PHONE.filter { it.always || it.action in acts }
    } else {
        val acts = Core.deviceStatus[node]?.actions
        DEVICE.filter { it.always || acts?.contains(it.action) == true }
    }

    /** The preset [a] was made from, if any. */
    fun of(a: Automation): AutoPreset? {
        val list = if (a.node.isEmpty()) PHONE else DEVICE
        return list.firstOrNull { p ->
            p.action == a.action && (p.action == SEND_TEXT || sameArgs(p.args, a.args))
        }
    }

    private fun sameArgs(x: String, y: String): Boolean = runCatching {
        val a = JSONObject(x)
        val b = JSONObject(y)
        a.length() == b.length() && a.keys().asSequence().all { k -> b.has(k) && a.get(k) == b.get(k) }
    }.getOrDefault(false)
}

/**
 * The phone's automations: "when this happens, do that", e.g. "when my
 * computer connects, set the ringer to silent" or "when this phone leaves
 * Wi-Fi, lock my computer". Set up in Settings (Automations).
 *
 *  - kept in `automations.json` in the app's files dir; the rules (what
 *    matches, once a minute, the JSON) are [AutomationBook];
 *  - what starts one: a device's link coming up, or staying down for a
 *    minute ([Core.links]); this phone's battery dropping below a level,
 *    starting or stopping charging (NetService's battery watch); joining or
 *    leaving Wi-Fi (a network callback). A network's name needs the
 *    location permission, which is never asked for here: without it, only
 *    "any Wi-Fi" can be chosen;
 *  - what it does: a preset on this phone ([Controls.run]) or another
 *    device ([Core.runAction], [Core.ring], [Core.sendText]), or just a
 *    notification; a notification says each time one runs;
 *  - only while NetService runs ([start] / [stop]).
 *
 * Everything here runs on the main thread, except the actions themselves.
 */
object PhoneAutomations {
    private const val TAG = "mydomain_auto"
    private const val FILE = "automations.json"
    const val CHANNEL = "automations"
    /** How often the links are looked at ("disconnects" waits a minute). */
    private const val TICK_MS = 15_000L
    /** Wi-Fi's name often arrives a moment after the network. */
    private const val WIFI_SETTLE_MS = 2_000L

    /** The automations, for the screen (main thread). */
    val items = mutableStateListOf<Automation>()

    private val main = Handler(Looper.getMainLooper())
    private val io = Executors.newSingleThreadExecutor()
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
    private val noteId = AtomicInteger(9100)

    private var book: AutomationBook? = null
    private var file: File? = null
    private var app: Context? = null
    private var running = false
    private var wifiCallback: ConnectivityManager.NetworkCallback? = null
    /** Wi-Fi networks up now (main thread). */
    private val wifiNets = HashSet<Network>()

    /** Read the saved automations, once. */
    fun load(ctx: Context): AutomationBook {
        book?.let { return it }
        val c = ctx.applicationContext
        app = c
        val f = File(c.filesDir, FILE)
        file = f
        val list = runCatching { if (f.exists()) AutomationBook.decode(f.readText()) else emptyList() }
            .onFailure { Log.w(TAG, "couldn't read the automations", it) }
            .getOrDefault(emptyList())
        val b = AutomationBook(list)
        book = b
        items.clear()
        items.addAll(b.list)
        return b
    }

    // --- the service ---------------------------------------------------------

    /** Start watching (NetService.onCreate). */
    fun start(ctx: Context) {
        val c = ctx.applicationContext
        val b = load(c)
        createChannel(c)
        if (running) return
        running = true
        // A fresh look at everything: what changed while stopped isn't news.
        book = AutomationBook(b.list)
        watchWifi(c)
        main.postDelayed(tick, TICK_MS)
        main.post { checkLinks() }
    }

    /** Stop watching (NetService.onDestroy). */
    fun stop(ctx: Context) {
        running = false
        main.removeCallbacks(tick)
        main.removeCallbacks(wifiCheck)
        wifiCallback?.let { cb ->
            runCatching { ctx.applicationContext.getSystemService(ConnectivityManager::class.java)?.unregisterNetworkCallback(cb) }
        }
        wifiCallback = null
        wifiNets.clear()
    }

    /** This phone's battery (NetService, main thread). */
    fun onBattery(pct: Int, charging: Boolean) {
        if (!running) return
        val b = book ?: return
        fire(b.battery(pct, charging))
    }

    /** A link came up or went down, or the devices changed (core events,
     *  any thread): look now, after [Core] has applied it. */
    fun linksChanged() {
        main.post { if (running) checkLinks() }
    }

    private val tick = object : Runnable {
        override fun run() {
            if (!running) return
            checkLinks()
            checkWifi()
            main.postDelayed(this, TICK_MS)
        }
    }

    private fun checkLinks() {
        val b = book ?: return
        val me = Core.me?.nodeId
        val nodes = Core.peers.map { it.nodeId }.filter { it != me }
        fire(b.links(nodes, Core.links.keys.toSet(), System.currentTimeMillis()))
    }

    private val wifiCheck = Runnable { checkWifi() }

    private fun wifiSoon() {
        main.removeCallbacks(wifiCheck)
        main.postDelayed(wifiCheck, WIFI_SETTLE_MS)
    }

    private fun checkWifi() {
        if (!running) return
        val b = book ?: return
        val c = app ?: return
        val connected = wifiNets.isNotEmpty()
        fire(b.wifi(connected, if (connected) currentSsid(c) else null))
    }

    private fun watchWifi(c: Context) {
        val cm = c.getSystemService(ConnectivityManager::class.java) ?: return
        val cb = object : ConnectivityManager.NetworkCallback() {
            override fun onAvailable(network: Network) {
                main.post { wifiNets.add(network); wifiSoon() }
            }

            override fun onLost(network: Network) {
                main.post { wifiNets.remove(network); wifiSoon() }
            }

            override fun onCapabilitiesChanged(network: Network, caps: NetworkCapabilities) {
                main.post { wifiSoon() }
            }
        }
        val req = NetworkRequest.Builder().addTransportType(NetworkCapabilities.TRANSPORT_WIFI).build()
        runCatching { cm.registerNetworkCallback(req, cb) }
            .onSuccess { wifiCallback = cb }
            .onFailure { Log.w(TAG, "can't watch Wi-Fi", it) }
        // The first look (connected or not) only sets where things stand.
        wifiSoon()
    }

    /** May the app read Wi-Fi names? (Android ties it to location.) Never
     *  asked for here. */
    fun canSeeWifiName(ctx: Context): Boolean =
        ctx.checkSelfPermission(Manifest.permission.ACCESS_FINE_LOCATION) == PackageManager.PERMISSION_GRANTED ||
            ctx.checkSelfPermission(Manifest.permission.ACCESS_COARSE_LOCATION) == PackageManager.PERMISSION_GRANTED

    /** The Wi-Fi network's name, if Android tells it. */
    @Suppress("DEPRECATION")
    fun currentSsid(ctx: Context): String? {
        if (!canSeeWifiName(ctx)) return null
        val wm = ctx.applicationContext.getSystemService(WifiManager::class.java) ?: return null
        val s = runCatching { wm.connectionInfo?.ssid }.getOrNull()?.removeSurrounding("\"") ?: return null
        return s.takeIf { it.isNotBlank() && it != "<unknown ssid>" && it != "0x" }
    }

    // --- running them ----------------------------------------------------------

    private fun fire(edges: List<Edge>) {
        if (edges.isEmpty()) return
        val b = book ?: return
        val now = System.currentTimeMillis()
        val due = edges.flatMap { b.take(it, now) }
        if (due.isEmpty()) return
        changed()
        due.forEach { perform(it) }
    }

    private fun perform(a: Automation) {
        val c = app ?: return
        Core.labLog("automation ${a.id} ${a.trigger.kind} -> ${a.action}")
        scope.launch {
            val result = runCatching { act(c, a) }
            tell(c, a, result)
        }
    }

    /** Do what [a] says; the transport used, "queued", or "here". Also
     *  for an automation app asking to run one now (AutomationAppsReceiver). */
    suspend fun act(c: Context, a: Automation): String {
        val args = runCatching { JSONObject(a.args) }.getOrDefault(JSONObject())
        return when {
            a.action == AutomationBook.NOTIFY -> "here"
            a.node.isEmpty() -> {
                if (!Controls.run(c, a.action, args)) throw IllegalStateException(c.getString(R.string.automation_not_done))
                "here"
            }
            a.action == "ring" -> Core.ring(a.node)
            a.action == AutoPresets.SEND_TEXT -> Core.sendText(a.node, args.optString("text"), "notify")
            else -> Core.runAction(a.node, a.action, args)
        }
    }

    /** A notification saying it ran, and how it went. */
    private fun tell(c: Context, a: Automation, result: Result<String>) {
        val what = thenText(c, a)
        val body = result.fold(
            { via ->
                when {
                    a.action == AutomationBook.NOTIFY -> c.getString(R.string.automation_note_notify)
                    via == "queued" -> c.getString(R.string.automation_note_queued, what)
                    else -> c.getString(R.string.automation_note_ran, what)
                }
            },
            { e -> c.getString(R.string.automation_note_failed, what, friendlyError(e)) },
        )
        val open = Intent(c, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_SINGLE_TOP)
        val id = noteId.incrementAndGet()
        val pi = PendingIntent.getActivity(c, id, open, PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
        val n = NotificationCompat.Builder(c, CHANNEL)
            .setSmallIcon(android.R.drawable.ic_dialog_info)
            .setContentTitle(whenText(c, a.trigger))
            .setContentText(body)
            .setStyle(NotificationCompat.BigTextStyle().bigText(body))
            .setPriority(NotificationCompat.PRIORITY_DEFAULT)
            .setAutoCancel(true)
            .setContentIntent(pi)
            .build()
        runCatching { c.getSystemService(NotificationManager::class.java)?.notify(id, n) }
    }

    fun createChannel(c: Context) {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.O) return
        val ch = NotificationChannel(CHANNEL, c.getString(R.string.channel_automations), NotificationManager.IMPORTANCE_DEFAULT)
        runCatching { c.getSystemService(NotificationManager::class.java)?.createNotificationChannel(ch) }
    }

    // --- words -------------------------------------------------------------------

    /** What starts it, as a sentence start ("When MacBook connects"). */
    fun whenText(c: Context, t: Trigger): String = when (t.kind) {
        TriggerKind.CONNECTS -> c.getString(R.string.automation_when_connects, Core.peerName(t.node))
        TriggerKind.DISCONNECTS -> c.getString(R.string.automation_when_disconnects, Core.peerName(t.node))
        TriggerKind.BATTERY_BELOW -> c.getString(R.string.automation_when_battery_below, t.level ?: 0)
        TriggerKind.CHARGING_STARTS -> c.getString(R.string.automation_when_charging_starts)
        TriggerKind.CHARGING_STOPS -> c.getString(R.string.automation_when_charging_stops)
        TriggerKind.WIFI_JOINS ->
            if (t.ssid.isEmpty()) c.getString(R.string.automation_when_wifi_joins_any)
            else c.getString(R.string.automation_when_wifi_joins, t.ssid)
        TriggerKind.WIFI_LEAVES ->
            if (t.ssid.isEmpty()) c.getString(R.string.automation_when_wifi_leaves_any)
            else c.getString(R.string.automation_when_wifi_leaves, t.ssid)
        else -> t.kind
    }

    /** What it does ("Lock it on MacBook", "Tell me"). */
    fun thenText(c: Context, a: Automation): String {
        if (a.action == AutomationBook.NOTIFY) return c.getString(R.string.automation_then_notify)
        val where = if (a.node.isEmpty()) c.getString(R.string.automation_this_phone) else Core.peerName(a.node)
        return c.getString(R.string.automation_then, a.label.ifEmpty { a.action }, where)
    }

    // --- changes from the screen (main thread) -------------------------------------

    /** Add [a]; null, or why it couldn't be. */
    fun add(ctx: Context, a: Automation): String? = change(ctx) { it.add(a) }

    /** Change [a] (when it last ran is kept); null, or why not. */
    fun update(ctx: Context, a: Automation): String? = change(ctx) { it.update(a) }

    fun setEnabled(ctx: Context, id: String, on: Boolean) {
        change(ctx) { it.setEnabled(id, on) }
    }

    fun delete(ctx: Context, id: String) {
        change(ctx) { it.delete(id) }
    }

    private fun change(ctx: Context, block: (AutomationBook) -> Unit): String? {
        val b = load(ctx)
        return try {
            block(b)
            changed()
            null
        } catch (e: IllegalArgumentException) {
            e.message ?: e.toString()
        }
    }

    /** Show and save the list as it is now. */
    private fun changed() {
        val b = book ?: return
        items.clear()
        items.addAll(b.list)
        val f = file ?: return
        val text = b.encode()
        io.execute {
            // Write beside it, then swap it in, so a crash can't leave half a file.
            runCatching {
                val tmp = File(f.path + ".tmp")
                tmp.writeText(text)
                if (!tmp.renameTo(f)) {
                    f.writeText(text)
                    tmp.delete()
                }
            }.onFailure { Log.w(TAG, "couldn't save the automations", it) }
        }
    }
}
