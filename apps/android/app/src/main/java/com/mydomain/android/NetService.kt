package com.mydomain.android

import android.app.*
import android.content.*
import android.content.pm.ServiceInfo
import android.net.ConnectivityManager
import android.net.Network
import android.net.wifi.WifiManager
import android.os.*
import android.provider.Settings
import android.telephony.TelephonyManager
import android.util.Log
import androidx.core.app.NotificationCompat
import androidx.core.content.ContextCompat
import com.mydomain.android.core.NetworkingUtils.wifiIpv4
import com.mydomain.android.core.ClipPrivacy
import com.mydomain.android.core.Core
import com.mydomain.android.core.KeystoreSecrets
import com.mydomain.android.core.Tone
import com.mydomain.android.ffi.EventListener
import com.mydomain.android.ffi.MdEngine
import kotlinx.coroutines.*
import org.json.JSONArray
import org.json.JSONObject

class NetService : Service() {
    private val serviceJob = SupervisorJob()
    private val serviceScope = CoroutineScope(Dispatchers.IO + serviceJob)
    
    private lateinit var clipboard: ClipboardManager
    private var multicastLock: WifiManager.MulticastLock? = null
    private var connectivityManager: ConnectivityManager? = null
    private var netCallback: ConnectivityManager.NetworkCallback? = null
    private var batteryReceiver: BroadcastReceiver? = null
    /** Last status pushed to the core, so only real changes are pushed. */
    private var lastStatus: String? = null
    /** When the phone started, unix seconds (for the status). */
    private val bootAt = (System.currentTimeMillis() - SystemClock.elapsedRealtime()) / 1000
    
    private val binder = LocalBinder()
    
    inner class LocalBinder : Binder() {
        fun getService(): NetService = this@NetService
    }

    override fun onBind(intent: Intent): IBinder = binder

    override fun onCreate() {
        super.onCreate()
        Core.init(this)
        clipboard = getSystemService(CLIPBOARD_SERVICE) as ClipboardManager
        
        val wifi = applicationContext.getSystemService(WIFI_SERVICE) as WifiManager
        multicastLock = wifi.createMulticastLock("mydomain").apply { 
            setReferenceCounted(true)
            acquire() 
        }

        connectivityManager = getSystemService(CONNECTIVITY_SERVICE) as ConnectivityManager
        netCallback = object : ConnectivityManager.NetworkCallback() {
            override fun onAvailable(network: Network) { networkChanged() }
            override fun onLost(network: Network) { networkChanged() }
        }
        runCatching { connectivityManager?.registerDefaultNetworkCallback(netCallback!!) }

        createNotificationChannel()
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
            startForeground(1, createNotification(), ServiceInfo.FOREGROUND_SERVICE_TYPE_DATA_SYNC)
        } else {
            startForeground(1, createNotification())
        }

        // Restore the clipboard auto-sync ("active" mode) toggle.
        Core.enableClipSync(getSharedPreferences("md", MODE_PRIVATE).getBoolean("clip_sync", false))

        // Initialize Rust core if not already done. Prefer the IMEI as a stable
        // device id; on Android 10+ that's unreadable for normal apps, so fall
        // back to ANDROID_ID (also stable across restarts/reinstalls).
        Core.labLogFile = java.io.File(filesDir, "events.log")
        Core.labLog("service start")
        // A fresh engine (new process): copies of files being sent by an
        // earlier process are orphans (their transfers died with it).
        if (Core.engine() == null) com.mydomain.android.core.OutgoingFiles.clearAll(this)
        runCatching { MdEngine.start(resolveDeviceId(), Build.MODEL ?: "android", wifiIpv4(), filesDir.absolutePath, coreEvents, KeystoreSecrets(this)) }
            .onFailure { Log.e("NetService", "engine failed to start", it) }
        // Received files land in an app-private folder, then move to Downloads.
        Core.engine()?.setDownloadDir(FileActions.downloadDir(this).absolutePath)
        // Folders your other devices may browse and download from.
        FileActions.shareFolders(this)
        // What's playing here, for your other devices (needs notification access).
        NowPlaying.start(this)
        // Your computers may look at the photos (we answer "no access" until
        // it's allowed), and the photo backup watches for new ones.
        Core.engine()?.setGalleryEnabled(true)
        // A computer's sound plays here (any Android: ComputerAudio.kt);
        // this phone's audio on a computer: Android 10 or later.
        Core.engine()?.setAudioRoles(true, PhoneAudio.supported)
        // Labels given on this phone before they were shared.
        Core.engine()?.let { com.mydomain.android.core.DeviceLabels.seed(it) }
        PhotoBackup.start(this)
        // Save battery: heavy background work waits while the battery is low,
        // Battery Saver is on or the phone dozes (and big transfers on mobile data).
        PowerSaver.start(this) { serviceScope.launch { pushStatus() } }
        // Started by a push with no UI showing: background until it appears.
        Core.engine()?.setBackground(!Core.visible)
        Core.reload()
        Fcm.init(this)

        publishInstalledApps()
        Controls.onChange = { serviceScope.launch { pushStatus() } }
        Controls.init(this)
        Radios.init(this)
        ScreenSettings.init(this)
        WifiNetworks.init(this)
        // Lost mode survives a restart of the app: its screen comes back now
        // and each time the screen turns on, until it's turned off.
        LostMode.watchScreen(this)
        LostMode.show(this, Core.visible || PhoneHelper.instance != null)
        // Automations watch the links, the battery and Wi-Fi from here on
        // (before the battery watch, whose first value only sets where things stand).
        PhoneAutomations.start(this)
        watchBattery()
        // Your computers hear when this phone goes into Do Not Disturb.
        DndWatch.start(this)
        startNetworking()
        BleLink.start(this)
    }

    /**
     * Keep the core's device status (asked for by `request_status`) current:
     * battery percent and charging. ACTION_BATTERY_CHANGED is sticky (the
     * current value arrives at once) and fires often (voltage, temperature),
     * so only a changed percent or charging state is pushed.
     */
    private fun watchBattery() {
        val r = object : BroadcastReceiver() {
            override fun onReceive(context: Context, intent: Intent) = pushBattery(intent)
        }
        batteryReceiver = r
        runCatching { registerReceiver(r, IntentFilter(Intent.ACTION_BATTERY_CHANGED)) }
            .onSuccess { sticky -> sticky?.let(::pushBattery) }
            .onFailure { Log.w("NetService", "battery watch failed", it) }
    }

    private fun pushBattery(intent: Intent) {
        val level = intent.getIntExtra(BatteryManager.EXTRA_LEVEL, -1)
        val scale = intent.getIntExtra(BatteryManager.EXTRA_SCALE, -1)
        if (level < 0 || scale <= 0) return
        val pct = (level * 100 / scale).coerceIn(0, 100)
        val st = intent.getIntExtra(BatteryManager.EXTRA_STATUS, -1)
        val charging = st == BatteryManager.BATTERY_STATUS_CHARGING || st == BatteryManager.BATTERY_STATUS_FULL
        battery = pct to charging
        PhoneAutomations.onBattery(pct, charging)
        // Tenths of a degree; whole degrees are plenty (and change less).
        val temp = intent.getIntExtra(BatteryManager.EXTRA_TEMPERATURE, Int.MIN_VALUE)
        batteryTemp = if (temp == Int.MIN_VALUE) null else Math.round(temp / 10f)
        batteryHealth = when (intent.getIntExtra(BatteryManager.EXTRA_HEALTH, -1)) {
            BatteryManager.BATTERY_HEALTH_GOOD -> "good"
            BatteryManager.BATTERY_HEALTH_OVERHEAT -> "overheat"
            BatteryManager.BATTERY_HEALTH_DEAD -> "dead"
            BatteryManager.BATTERY_HEALTH_OVER_VOLTAGE -> "over_voltage"
            BatteryManager.BATTERY_HEALTH_COLD -> "cold"
            BatteryManager.BATTERY_HEALTH_UNSPECIFIED_FAILURE -> "failure"
            else -> null
        }
        PhotoBackup.setCharging(charging)
        PowerSaver.onBattery(pct, charging)
        pushStatus()
    }

    private var battery: Pair<Int, Boolean>? = null
    private var batteryTemp: Int? = null
    private var batteryHealth: String? = null

    /** Wi-Fi on, connected, its name if Android tells it (it needs the
     *  location permission, which this app doesn't ask for, so usually not)
     *  and signal 0-4. */
    @Suppress("DEPRECATION")
    private fun wifiStatus(): JSONObject? = runCatching {
        val wm = applicationContext.getSystemService(WifiManager::class.java) ?: return@runCatching null
        val cm = getSystemService(ConnectivityManager::class.java)
        val caps = cm?.activeNetwork?.let { cm.getNetworkCapabilities(it) }
        val connected = caps?.hasTransport(android.net.NetworkCapabilities.TRANSPORT_WIFI) == true
        val o = JSONObject().put("on", wm.isWifiEnabled).put("connected", connected)
        if (connected) {
            val info = wm.connectionInfo
            val ssid = info?.ssid?.removeSurrounding("\"")
            if (!ssid.isNullOrBlank() && ssid != "<unknown ssid>") o.put("ssid", ssid)
            info?.rssi?.let { rssi ->
                val level = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) wm.calculateSignalLevel(rssi) * 4 / (wm.maxSignalLevel.coerceAtLeast(1))
                else WifiManager.calculateSignalLevel(rssi, 5)
                o.put("signal", level.coerceIn(0, 4))
            }
        }
        o
    }.getOrNull()

    /** Bluetooth adapter on or off (no permission needed to ask). */
    private fun bluetoothOn(): Boolean? = runCatching {
        getSystemService(android.bluetooth.BluetoothManager::class.java)?.adapter?.isEnabled
    }.getOrNull()

    /** Free and total memory, rounded to 100 MB (it moves all the time). */
    private fun ram(): JSONObject? = runCatching {
        val am = getSystemService(ActivityManager::class.java) ?: return@runCatching null
        val mi = ActivityManager.MemoryInfo().also { am.getMemoryInfo(it) }
        val step = 100L * 1024L * 1024L
        JSONObject().put("free", mi.availMem / step * step).put("total", mi.totalMem)
    }.getOrNull()

    /** Free and total bytes of the phone's storage (whole MB, so it doesn't
     *  change on every write), for the status. */
    private fun storage(): JSONObject? = runCatching {
        val fs = StatFs(Environment.getDataDirectory().path)
        val mb = 1024L * 1024L
        JSONObject().put("free", fs.availableBytes / mb * mb).put("total", fs.totalBytes / mb * mb)
    }.getOrNull()

    /** Battery, the controls' state and the actions we take, for devices
     *  that ask (`request_status`). Pushed only when it changed. */
    private fun pushStatus() {
        val o = Controls.state(this).put("actions", Controls.actions())
            .put("search_apps", DeviceActions.searchApps(this))
        battery?.let { (pct, charging) -> o.put("battery", pct).put("charging", charging) }
        storage()?.let { o.put("storage", it) }
        batteryTemp?.let { o.put("battery_temp", it) }
        batteryHealth?.let { o.put("battery_health", it) }
        wifiStatus()?.let { o.put("wifi", it) }
        bluetoothOn()?.let { o.put("bluetooth", it) }
        // Heavy background work waits to save battery (the computer shows it).
        if (PowerSaver.current.saving) o.put("saving", true)
        ram()?.let { o.put("ram", it) }
        // When it started (fixed, so it doesn't make every push different);
        // the core turns it into `uptime_s` when it answers. No `cpu`: apps
        // can't read /proc/stat since Android 8.
        o.put("boot_at", bootAt)
        val json = o.toString()
        if (json == lastStatus) return
        val engine = Core.engine() ?: return
        engine.setDeviceStatus(json)
        lastStatus = json
    }

    /**
     * Best-effort stable device id. Tries the IMEI (requires READ_PHONE_STATE and
     * is only obtainable by privileged apps on API 29+), and falls back to
     * ANDROID_ID — Google's official, permission-free device-id replacement.
     */
    private fun resolveDeviceId(): String {
        imei()?.takeIf { it.isNotBlank() }?.let { return it }
        return Settings.Secure.getString(contentResolver, Settings.Secure.ANDROID_ID) ?: ""
    }

    @Suppress("HardwareIds", "DEPRECATION", "MissingPermission")
    private fun imei(): String? {
        if (ContextCompat.checkSelfPermission(this, android.Manifest.permission.READ_PHONE_STATE)
            != android.content.pm.PackageManager.PERMISSION_GRANTED
        ) return null
        val tm = getSystemService(Context.TELEPHONY_SERVICE) as? TelephonyManager ?: return null
        return runCatching {
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) tm.imei else tm.deviceId
        }.getOrNull()
    }

    private fun createNotificationChannel() {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            val channel = NotificationChannel(
                "net_service", getString(R.string.channel_service),
                NotificationManager.IMPORTANCE_LOW
            )
            val shared = NotificationChannel(
                "shared_notif", getString(R.string.channel_shared),
                NotificationManager.IMPORTANCE_DEFAULT
            )
            val manager = getSystemService(NotificationManager::class.java)
            manager?.createNotificationChannel(channel)
            manager?.createNotificationChannel(shared)
        }
        DeviceActions.createChannels(this)
        FileActions.createChannels(this)
        ComputerNotifs.createChannel(this)
        NoteReminders.createChannel(this)
    }

    private fun createNotification(): Notification {
        val intent = Intent(this, MainActivity::class.java)
        val pendingIntent = PendingIntent.getActivity(this, 0, intent, PendingIntent.FLAG_IMMUTABLE)
        
        return NotificationCompat.Builder(this, "net_service")
            .setContentTitle(getString(R.string.service_running))
            .setContentText(getString(R.string.service_running_body))
            .setSmallIcon(android.R.drawable.ic_menu_share)
            .setOngoing(true)
            .setContentIntent(pendingIntent)
            .build()
    }

    /** Publish launchable apps (pkg + label) so peers can pick which to subscribe to. */
    private fun publishInstalledApps() {
        serviceScope.launch {
            val json = runCatching {
                val pm = packageManager
                val launchable = pm.getInstalledApplications(0)
                    .filter { pm.getLaunchIntentForPackage(it.packageName) != null }
                    .map { it.packageName to pm.getApplicationLabel(it).toString() }
                    .sortedBy { it.second.lowercase() }
                val arr = JSONArray()
                for ((pkg, label) in launchable) {
                    arr.put(JSONObject().put("pkg", pkg).put("label", label))
                }
                arr.toString()
            }.getOrDefault("[]")
            Core.engine()?.setInstalledApps(json)
        }
    }

    private val sharedNotifId = java.util.concurrent.atomic.AtomicInteger(1000)

    /** Post a peer-shared notification/call as a local OS notification. Requires
     *  POST_NOTIFICATIONS on API 33+; silently no-ops if not granted. */
    private fun postSharedNotification(title: String, text: String) {
        if (title.isBlank() && text.isBlank()) return
        val n = NotificationCompat.Builder(this, "shared_notif")
            .setContentTitle(title.ifBlank { getString(R.string.notification_default_title) })
            .setContentText(text)
            .setSmallIcon(android.R.drawable.ic_dialog_info)
            .setAutoCancel(true)
            .build()
        runCatching { getSystemService(NotificationManager::class.java)?.notify(sharedNotifId.incrementAndGet(), n) }
    }

    /**
     * Core events, pushed by the Rust core as they happen (UniFFI callback,
     * on core threads). The UI state lives in [Core]; the service adds what
     * needs Android: OS notifications and the clipboard.
     */
    private val coreEvents = object : EventListener {
        override fun onEvent(name: String, payloadJson: String) {
            // A computer's sound, many frames a second: straight to the
            // player, never logged or kept.
            if (name == "audio-frame") {
                ComputerAudio.onFrame(payloadJson)
                return
            }
            Core.onEvent(name, payloadJson)
            // A screen snapshot is for the UI alone (and big): don't parse it again.
            if (name == "screen-frame") return
            serviceScope.launch {
                try {
                    val o = JSONObject(payloadJson)
                    when (o.optString("kind")) {
                        "message_received" -> handleIncomingMessage(o)
                        // A computer's own notification (it was asked for them), or
                        // one a device shared by hand / from a phone app.
                        "notification" ->
                            if (ComputerNotifs.handles(this@NetService, o)) ComputerNotifs.post(this@NetService, o)
                            else postSharedNotification(o.optString("title"), o.optString("body"))
                        "notification_removed" -> ComputerNotifs.remove(this@NetService, o.optString("from"), o.optString("key"))
                        // A computer that can share its notifications says so here.
                        "device_status" -> ComputerNotifs.onStatus(this@NetService, o.optString("from"), o.optJSONObject("status") ?: JSONObject())
                        "notification_action" -> NotifListener.onAction(o)
                        "call_notification" -> postSharedNotification(getString(R.string.notification_call, o.optString("caller")), o.optString("state"))
                        "ring" -> DeviceActions.ring(this@NetService, Core.peerName(o.optString("from")))
                        "open_url" -> DeviceActions.openUrl(this@NetService, Core.peerName(o.optString("from")), o.optString("url"), o.optString("app").ifEmpty { null })
                        "action" -> Controls.run(this@NetService, o.optString("name"), o.optJSONObject("args") ?: JSONObject(), Core.peerName(o.optString("from")))
                        "file_progress" -> FileActions.onProgress(this@NetService, o)
                        "file_received" -> FileActions.onReceived(this@NetService, o)
                        // A computer asks to sync a folder: say so.
                        "sync_pairs" -> SyncFolders.onPairs(this@NetService, Core.engine()?.syncPairs().orEmpty())
                        "file_failed" -> FileActions.onFailed(this@NetService, o)
                        "text_shared" -> onTextShared(o)
                        "gallery_request" -> Gallery.onRequest(this@NetService, o)
                        "screen_watch" -> ScreenShare.onWatch(this@NetService, o.optString("from"), o.optBoolean("on"))
                        // Taps and swipes on a computer's picture of this screen.
                        "input" -> ScreenShare.onInput(this@NetService, o.optString("from"), o.optJSONArray("events"))
                        // A computer asks to play this phone's audio, or stopped listening.
                        "audio_listen" -> PhoneAudio.onListen(this@NetService, o.optString("from"), o.optBoolean("on"))
                        // A computer offers its sound here (the phone asks
                        // first), or stopped / said no.
                        "audio_offer" -> ComputerAudio.onOffer(this@NetService, o.optString("from"), o.optBoolean("on"))
                        "peer_disconnected" -> {
                            PhoneAudio.linkLost(o.optString("node_id"))
                            ComputerAudio.linkLost(o.optString("node_id"))
                            PhoneAutomations.linksChanged()
                        }
                        "peer_connected" -> {
                            PhoneAutomations.linksChanged()
                            // A computer may have restarted: ask for its notifications again.
                            ComputerNotifs.onConnected(this@NetService, o.optString("node_id"))
                        }
                        "peers_updated" -> PhoneAutomations.linksChanged()
                        "battery_alert" -> DeviceActions.batteryAlert(this@NetService, Core.peerName(o.optString("from")), o.optString("alert"), o.optInt("level"))
                    }
                } catch (e: Exception) {
                    Log.e("NetService", "Error handling core event $name", e)
                }
            }
        }
    }

    private fun networkChanged() {
        Core.engine()?.networkChanged(wifiIpv4())
        // Wi-Fi joined or left: the status shows it.
        pushStatus()
    }

    private fun startNetworking() {
        // No background reconnect loop: links open on demand (a send opens a
        // QUIC link), and peers reach us through a push when we have none.

        // Track the local clipboard. We always remember the latest value (so we
        // can answer passive "get clipboard" pulls), but only broadcast changes
        // to peers while ACTIVE auto-sync is enabled.
        Handler(Looper.getMainLooper()).post {
            clipboard.addPrimaryClipChangedListener {
                val clip = clipboard.primaryClip
                if (clip != null && clip.itemCount > 0) {
                    val text = clip.getItemAt(0).text?.toString()
                    // A copy marked sensitive (a password manager's, or the
                    // automation-apps key from Settings) stays on this phone,
                    // unless "Never sync passwords…" is off (ClipPrivacy).
                    val stayHere = ClipPrivacy.keepHere(ClipPrivacy.isSensitive(clip.description), ClipPrivacy.hideConcealed(this))
                    if (text != null && !stayHere && !text.startsWith("SYNC_IGNORE:")) { // Avoid loops if we use a prefix
                        currentLocalClip = text
                        // A copy made on this phone (not one we applied from
                        // a device, nor a recent value copied again).
                        val recopy = Core.consumeRecopy(text)
                        if (Core.clipSync && !recopy && text != lastReceivedClipboard) {
                            Core.recordClip(text, null)
                        }
                        if (Core.clipSync) syncClipboard(text)
                    }
                }
            }
        }
    }

    /** Text another device sent (`send_text`): onto the clipboard for
     *  "clipboard" (the core leaves that to us here), and a notice. */
    private fun onTextShared(o: JSONObject) {
        val text = o.optString("text")
        val from = o.optString("from")
        if (text.isBlank()) return
        val toClipboard = o.optString("mode") == "clipboard"
        // A password sent anyway: on the clipboard marked sensitive (the
        // keyboard and Android's preview hide it), not kept, not shown.
        val sensitive = o.optBoolean("sensitive")
        if (toClipboard) {
            // Not sent back out by the sync: it came from that device.
            lastReceivedClipboard = text
            updateLocalClipboard(text, sensitive)
            if (!sensitive) Core.recordClip(text, from.ifEmpty { null })
        }
        val shown = if (sensitive) getString(R.string.text_password_hidden) else text
        DeviceActions.sharedText(this, Core.peerName(from), shown, toClipboard)
    }

    private var lastReceivedClipboard: String? = null
    /** Latest known local clipboard text, used to answer passive pull requests. */
    private var currentLocalClip: String = ""

    private fun handleIncomingMessage(msg: JSONObject) {
        val text = msg.optString("text", "")
        val json = try { JSONObject(text) } catch (e: Exception) { return } // regular chat
        when (json.optString("type")) {
            // Unsolicited push from a peer in active mode: only apply if we also
            // have auto-sync enabled.
            "clipboard" -> {
                if (!Core.clipSync) return
                val content = json.optString("content")
                if (content != lastReceivedClipboard) {
                    lastReceivedClipboard = content
                    updateLocalClipboard(content)
                    Core.recordClip(content, msg.optString("node_id").ifEmpty { null })
                }
            }
            // A peer is pulling our clipboard: reply with the latest known value.
            "clipboard_request" -> {
                val requester = json.optString("from")
                if (requester.isNotEmpty()) {
                    val resp = JSONObject().apply {
                        put("type", "clipboard_response")
                        put("content", currentLocalClip)
                        put("from", Core.me?.nodeId ?: "")
                    }.toString()
                    sendClip(requester, resp)
                }
            }
            // Reply to a pull we initiated: always apply (it was solicited).
            "clipboard_response" -> {
                val content = json.optString("content")
                lastReceivedClipboard = content
                updateLocalClipboard(content)
                Core.recordClip(content, msg.optString("node_id").ifEmpty { json.optString("from") }.ifEmpty { null })
                Core.addActivity(getString(R.string.act_clipboard_copied, Core.peerName(msg.optString("node_id"))), Tone.Ok)
            }
        }
    }

    /** Send a clipboard control message to one peer over the active connection
     *  (over the live link; the engine opens a QUIC link if there is none). */
    private fun sendClip(nodeId: String, json: String) {
        serviceScope.launch {
            runCatching { Core.engine()?.send(nodeId, json) }
        }
    }

    private fun updateLocalClipboard(content: String, sensitive: Boolean = false) {
        Handler(Looper.getMainLooper()).post {
            try {
                // To avoid triggering our own listener and causing a loop, 
                // we could use a flag or check the content.
                val clip = ClipPrivacy.clipOf("Synced Clipboard", content, sensitive)
                clipboard.setPrimaryClip(clip)
                Log.i("NetService", "Clipboard updated from remote")
            } catch (_: Exception) {
                Log.e("NetService", "Failed to update clipboard")
            }
        }
    }

    private fun syncClipboard(content: String) {
        if (content == lastReceivedClipboard) return
        
        serviceScope.launch {
            val json = JSONObject().apply {
                put("type", "clipboard")
                put("content", content)
            }.toString()
            
            val engine = Core.engine() ?: return@launch
            for (p in engine.peers()) runCatching { engine.send(p.nodeId, json) }
        }
    }

    override fun onDestroy() {
        super.onDestroy()
        serviceJob.cancel()
        multicastLock?.let { if (it.isHeld) it.release() }
        netCallback?.let { runCatching { connectivityManager?.unregisterNetworkCallback(it) } }
        batteryReceiver?.let { runCatching { unregisterReceiver(it) } }
        BleLink.stop()
        PhotoBackup.stop()
        PowerSaver.stop()
        DndWatch.stop(this)
        PhoneAutomations.stop(this)
        LostMode.unwatchScreen(this)
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        return START_STICKY
    }
}
