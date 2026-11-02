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
    
    private val binder = LocalBinder()
    
    inner class LocalBinder : Binder() {
        fun getService(): NetService = this@NetService
    }

    override fun onBind(intent: Intent): IBinder = binder

    override fun onCreate() {
        super.onCreate()
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
        runCatching { MdEngine.start(resolveDeviceId(), Build.MODEL ?: "android", wifiIpv4(), filesDir.absolutePath, coreEvents, KeystoreSecrets(this)) }
            .onFailure { Log.e("NetService", "engine failed to start", it) }
        // Started by a push with no UI showing: background until it appears.
        Core.engine()?.setBackground(!Core.visible)
        Core.reload()
        Fcm.init(this)

        publishInstalledApps()
        startNetworking()
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
                "net_service", "Networking Service",
                NotificationManager.IMPORTANCE_LOW
            )
            val shared = NotificationChannel(
                "shared_notif", "Shared notifications",
                NotificationManager.IMPORTANCE_DEFAULT
            )
            val manager = getSystemService(NotificationManager::class.java)
            manager?.createNotificationChannel(channel)
            manager?.createNotificationChannel(shared)
        }
    }

    private fun createNotification(): Notification {
        val intent = Intent(this, MainActivity::class.java)
        val pendingIntent = PendingIntent.getActivity(this, 0, intent, PendingIntent.FLAG_IMMUTABLE)
        
        return NotificationCompat.Builder(this, "net_service")
            .setContentTitle("my-domain is running")
            .setContentText("Your devices can reach this phone.")
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
            .setContentTitle(title.ifBlank { "Notification" })
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
            Core.onEvent(name, payloadJson)
            serviceScope.launch {
                try {
                    val o = JSONObject(payloadJson)
                    when (o.optString("kind")) {
                        "message_received" -> handleIncomingMessage(o)
                        "notification" -> postSharedNotification(o.optString("title"), o.optString("body"))
                        "call_notification" -> postSharedNotification("Call · ${o.optString("caller")}", o.optString("state"))
                    }
                } catch (e: Exception) {
                    Log.e("NetService", "Error handling core event $name", e)
                }
            }
        }
    }

    private fun networkChanged() {
        Core.engine()?.networkChanged(wifiIpv4())
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
                    if (text != null && !text.startsWith("SYNC_IGNORE:")) { // Avoid loops if we use a prefix
                        currentLocalClip = text
                        if (Core.clipSync) syncClipboard(text)
                    }
                }
            }
        }
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
                Core.addActivity("Copied ${Core.peerName(msg.optString("node_id"))}'s clipboard", Tone.Ok)
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

    private fun updateLocalClipboard(content: String) {
        Handler(Looper.getMainLooper()).post {
            try {
                // To avoid triggering our own listener and causing a loop, 
                // we could use a flag or check the content.
                val clip = ClipData.newPlainText("Synced Clipboard", content)
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
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        return START_STICKY
    }
}
