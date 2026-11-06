package com.mydomain.android

import android.Manifest
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.content.pm.PackageManager
import android.location.LocationManager
import android.net.wifi.ScanResult
import android.net.wifi.WifiConfiguration
import android.net.wifi.WifiManager
import android.os.Build
import android.provider.Settings
import android.util.Log
import androidx.core.app.NotificationCompat
import com.mydomain.android.core.Core
import org.json.JSONArray
import org.json.JSONObject

/**
 * Nearby Wi-Fi networks for your computers, and joining one: the `wifi`
 * action.
 *
 *   wifi {op: "scan"}                       scan; the list goes in the status
 *   wifi {op: "connect", ssid, password?}   join (a saved network needs none)
 *   wifi {op: "disconnect" | "reconnect"}   leave / rejoin the current one
 *
 * All of it works because the app targets Android 9 (see Sdk.kt): newer
 * targets may only suggest networks. Android shows scan results only to
 * an app with the location permission, and only while Location is on
 * (Wi-Fi networks reveal where you are); the status says which is missing
 * (`wifi_scan_needs`), and a notification on the phone fixes it with a tap.
 * The location itself is never read.
 */
object WifiNetworks {
    private const val TAG = "mydomain_wifi"
    private const val MAX = 20
    private const val NOTE_ID = 7501
    const val EXTRA_ASK = "ask_wifi_scan"

    /** The last scan, for the status (null until the first scan). */
    @Volatile
    private var networks: JSONArray? = null

    /** What stops a scan: "permission", "location", or null. */
    @Volatile
    private var needs: String? = null
    private var receiver: BroadcastReceiver? = null

    fun init(ctx: Context) {
        if (receiver != null) return
        val r = object : BroadcastReceiver() {
            override fun onReceive(c: Context, i: Intent) {
                if (networks != null) {
                    networks = read(c)
                    Controls.onChange()
                }
            }
        }
        runCatching { ctx.applicationContext.registerReceiver(r, IntentFilter(WifiManager.SCAN_RESULTS_AVAILABLE_ACTION)) }
            .onSuccess { receiver = r }
    }

    /** Stop watching (NetService.onDestroy; [init] starts again). */
    fun stop(ctx: Context) {
        receiver?.let { runCatching { ctx.applicationContext.unregisterReceiver(it) } }
        receiver = null
    }

    /** The networks and what's missing, into the status. */
    fun state(o: JSONObject) {
        networks?.let { o.put("wifi_networks", it) }
        needs?.let { o.put("wifi_scan_needs", it) }
    }

    /** Run `wifi {op, ...}`; false if it couldn't be done. */
    @Suppress("DEPRECATION")
    fun run(ctx: Context, args: JSONObject): Boolean {
        val app = ctx.applicationContext
        val wm = app.getSystemService(WifiManager::class.java) ?: return false
        return when (args.optString("op")) {
            "scan" -> scan(app, wm)
            "disconnect" -> wm.disconnect()
            "reconnect" -> wm.reconnect()
            "connect" -> connect(app, wm, args.optString("ssid"), args.optString("password").takeIf { args.has("password") })
            else -> false
        }
    }

    @Suppress("DEPRECATION")
    private fun scan(app: Context, wm: WifiManager): Boolean {
        needs = missing(app)
        needs?.let {
            askFor(app, it)
            Controls.onChange()
            return false
        }
        if (!wm.isWifiEnabled) wm.isWifiEnabled = true
        // Show what Android already knows at once; the fresh scan follows.
        networks = read(app)
        Controls.onChange()
        return wm.startScan() || networks != null
    }

    /** Join [ssid]: a saved network by its id, a new one with [password]
     *  (WPA/WPA2; none for an open network). */
    @Suppress("DEPRECATION")
    private fun connect(app: Context, wm: WifiManager, ssid: String, password: String?): Boolean {
        if (ssid.isBlank() || ssid.length > 32) return false
        if (!wm.isWifiEnabled) wm.isWifiEnabled = true
        val quoted = "\"$ssid\""
        val saved = runCatching { wm.configuredNetworks?.firstOrNull { it.SSID == quoted } }.getOrNull()
        val id = when {
            saved != null && password == null -> saved.networkId
            else -> {
                if (password != null && password.isNotEmpty() && password.length !in 8..63) return false
                val conf = WifiConfiguration().apply {
                    SSID = quoted
                    if (password.isNullOrEmpty()) {
                        allowedKeyManagement.set(WifiConfiguration.KeyMgmt.NONE)
                    } else {
                        preSharedKey = "\"$password\""
                    }
                }
                saved?.let { wm.removeNetwork(it.networkId) }
                wm.addNetwork(conf)
            }
        }
        if (id < 0) return false
        Log.i(TAG, "joining $ssid")
        return wm.enableNetwork(id, true) && wm.reconnect()
    }

    /** The scan results: one entry per network name, strongest first. */
    @Suppress("DEPRECATION")
    private fun read(app: Context): JSONArray {
        val wm = app.getSystemService(WifiManager::class.java) ?: return JSONArray()
        val results: List<ScanResult> = runCatching { wm.scanResults }.getOrNull().orEmpty()
        val current = runCatching { wm.connectionInfo?.ssid?.removeSurrounding("\"") }.getOrNull()
        val saved = runCatching { wm.configuredNetworks?.map { it.SSID.removeSurrounding("\"") }?.toSet() }.getOrNull().orEmpty()
        val best = results.filter { it.SSID.isNotBlank() }
            .groupBy { it.SSID }
            .map { (_, same) -> same.maxBy { it.level } }
            .sortedByDescending { it.level }
            .take(MAX)
        return JSONArray().apply {
            for (r in best) {
                put(
                    JSONObject()
                        .put("ssid", r.SSID)
                        .put("signal", WifiManager.calculateSignalLevel(r.level, 5).coerceIn(0, 4))
                        .put("secure", listOf("WPA", "WEP", "SAE", "EAP").any { it in r.capabilities })
                        .put("saved", r.SSID in saved)
                        .put("current", r.SSID == current),
                )
            }
        }
    }

    /** What Android needs before it shows scan results to this app. */
    private fun missing(app: Context): String? {
        val granted = app.checkSelfPermission(Manifest.permission.ACCESS_FINE_LOCATION) == PackageManager.PERMISSION_GRANTED
        if (!granted) return "permission"
        val lm = app.getSystemService(LocationManager::class.java)
        val on = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.P) lm?.isLocationEnabled == true else true
        return if (on) null else "location"
    }

    /** Fix [need] on the phone: the app's permission prompt, or Location
     *  settings, opened straight away when Android lets this app open a
     *  screen (it's on screen, or its Accessibility helper is on: Android
     *  exempts those); else a notification that opens it with a tap. */
    private fun askFor(app: Context, need: String) {
        val open = if (need == "permission") {
            Intent(app, MainActivity::class.java).putExtra(EXTRA_ASK, true)
        } else {
            Intent(Settings.ACTION_LOCATION_SOURCE_SETTINGS)
        }.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
        if ((Core.visible || PhoneHelper.instance != null) && runCatching { app.startActivity(open) }.isSuccess) {
            Log.i(TAG, "opened the $need prompt")
            return
        }
        val pi = PendingIntent.getActivity(app, NOTE_ID, open, PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
        val n = NotificationCompat.Builder(app, DeviceActions.CH_LINKS)
            .setSmallIcon(android.R.drawable.ic_dialog_info)
            .setContentTitle(app.getString(R.string.wifi_scan_title))
            .setContentText(app.getString(if (need == "permission") R.string.wifi_scan_permission else R.string.wifi_scan_location))
            .setStyle(NotificationCompat.BigTextStyle().bigText(app.getString(if (need == "permission") R.string.wifi_scan_permission else R.string.wifi_scan_location)))
            .setAutoCancel(true)
            .setContentIntent(pi)
            .build()
        runCatching { app.getSystemService(NotificationManager::class.java)?.notify(NOTE_ID, n) }
    }
}
