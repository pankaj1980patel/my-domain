package com.mydomain.android

import android.content.Context
import android.database.ContentObserver
import android.net.ConnectivityManager
import android.net.Network
import android.net.NetworkCapabilities
import android.os.Handler
import android.os.Looper
import android.provider.MediaStore
import android.util.Log
import com.mydomain.android.core.Core
import com.mydomain.android.ffi.BackupMedia
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale

/**
 * Photo backup, the phone's part (the core decides what to send and when;
 * see mdcore `features::backup`). Runs inside [NetService]:
 *  - a ContentObserver on the photos and videos notices new ones, and a
 *    scan hands everything added since the backup was turned on to the
 *    core (it skips what it already sent or has waiting);
 *  - the conditions (unmetered Wi-Fi, charging) go to the core whenever
 *    they change, so it starts, pauses and resumes by itself.
 * No WorkManager: the service is already running, and the core's file
 * push resumes after a pause or a restart.
 */
object PhotoBackup {
    private const val TAG = "mydomain_backup"
    /** New photos often come in bursts (a few rows per shot): wait a moment. */
    private const val SCAN_DELAY_MS = 3_000L
    private const val PAGE = 500

    private val main = Handler(Looper.getMainLooper())
    private var observer: ContentObserver? = null
    private var netCallback: ConnectivityManager.NetworkCallback? = null
    private var app: Context? = null
    @Volatile private var charging = false
    private val scan = Runnable { Thread { scanNow() }.start() }

    fun start(ctx: Context) {
        val c = ctx.applicationContext
        app = c
        val o = object : ContentObserver(main) {
            override fun onChange(selfChange: Boolean) = scanSoon()
        }
        observer = o
        runCatching {
            c.contentResolver.registerContentObserver(MediaStore.Images.Media.EXTERNAL_CONTENT_URI, true, o)
            c.contentResolver.registerContentObserver(MediaStore.Video.Media.EXTERNAL_CONTENT_URI, true, o)
        }.onFailure { Log.w(TAG, "can't watch the gallery", it) }
        val cm = c.getSystemService(ConnectivityManager::class.java)
        val cb = object : ConnectivityManager.NetworkCallback() {
            override fun onAvailable(network: Network) = pushConditions()
            override fun onLost(network: Network) = pushConditions()
            override fun onCapabilitiesChanged(network: Network, caps: NetworkCapabilities) = pushConditions()
        }
        netCallback = cb
        runCatching { cm?.registerDefaultNetworkCallback(cb) }
        pushConditions()
        scanSoon()
    }

    fun stop() {
        val c = app ?: return
        observer?.let { runCatching { c.contentResolver.unregisterContentObserver(it) } }
        netCallback?.let { runCatching { c.getSystemService(ConnectivityManager::class.java)?.unregisterNetworkCallback(it) } }
        observer = null
        netCallback = null
        main.removeCallbacks(scan)
    }

    /** The battery's charging state changed (from NetService's battery watch). */
    fun setCharging(on: Boolean) {
        if (on == charging) return
        charging = on
        pushConditions()
    }

    /** On Wi-Fi (or Ethernet) that isn't metered. */
    private fun unmetered(c: Context): Boolean {
        val cm = c.getSystemService(ConnectivityManager::class.java) ?: return false
        val caps = cm.getNetworkCapabilities(cm.activeNetwork ?: return false) ?: return false
        val local = caps.hasTransport(NetworkCapabilities.TRANSPORT_WIFI) || caps.hasTransport(NetworkCapabilities.TRANSPORT_ETHERNET)
        return local && caps.hasCapability(NetworkCapabilities.NET_CAPABILITY_NOT_METERED)
    }

    private fun pushConditions() {
        val c = app ?: return
        val engine = Core.engine() ?: return
        runCatching { engine.backupConditions(unmetered(c), charging) }
    }

    fun scanSoon() {
        main.removeCallbacks(scan)
        main.postDelayed(scan, SCAN_DELAY_MS)
    }

    /** Turn the backup on or off and choose the computer; then look for
     *  photos to send. Throws the core's error (e.g. no computer chosen). */
    fun configure(enabled: Boolean, target: String, chargingOnly: Boolean) {
        val engine = Core.engine() ?: throw IllegalStateException("not started")
        engine.backupConfigure(enabled, target, chargingOnly)
        pushConditions()
        if (enabled) scanSoon()
    }

    private val month = SimpleDateFormat("yyyy-MM", Locale.US)

    /** Hand the core every photo/video added since the backup was turned on. */
    private fun scanNow() {
        val c = app ?: return
        val engine = Core.engine() ?: return
        val state = runCatching { engine.backupState() }.getOrNull() ?: return
        if (!state.enabled || !Gallery.canRead(c)) return
        val since = state.since.toLong().toString()
        var offset = 0
        var added = 0
        while (true) {
            val page = runCatching {
                Gallery.query(c, "date_added >= ?", arrayOf(since), newestFirst = false, limit = PAGE, offset = offset)
            }.onFailure { Log.w(TAG, "scan failed", it) }.getOrNull() ?: return
            val items = page.filter { it.path.isNotEmpty() && it.size > 0 }.map { m ->
                BackupMedia(
                    key = "${m.id}:${m.added}",
                    path = m.path,
                    taken = m.taken.coerceAtLeast(0).toULong(),
                    month = synchronized(month) { month.format(Date(m.taken * 1000)) },
                )
            }
            if (items.isNotEmpty()) added += runCatching { engine.backupEnqueue(items).toInt() }.getOrDefault(0)
            if (page.size < PAGE) break
            offset += PAGE
        }
        if (added > 0) Log.i(TAG, "$added new photo(s) for the backup")
    }
}
