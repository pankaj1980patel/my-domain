package com.mydomain.android

import android.annotation.SuppressLint
import android.content.Context
import android.net.wifi.WifiManager
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.util.Log
import com.mydomain.android.core.WifiDirectPlan
import java.net.Inet4Address
import java.net.NetworkInterface
import com.mydomain.android.core.ResourceBook
import com.mydomain.android.core.ResourceLedger

/**
 * This phone's local-only hotspot for a big file with a computer that can't
 * do Wi-Fi Direct (mdcore `features::wifi_direct`, method "hotspot"): the
 * core asks to "host"; the hotspot starts (`WifiManager
 * .startLocalOnlyHotspot`: no internet sharing, no WRITE_SETTINGS), its
 * name and passphrase go to the core (which sends them, encrypted, to the
 * computer over the link it has: the network, the relay, or Bluetooth when
 * there's nothing else) and are never logged or kept here; "leave" stops
 * it. It needs the same permission as Wi-Fi Direct (asked for by
 * [WifiDirect]).
 *
 * Some phones drop their own Wi-Fi while the hotspot runs: the core sends
 * the offer reliably, so it goes once a link is back (mobile data, the
 * relay, Bluetooth).
 */
object PhoneHotspot {
    private const val TAG = "mydomain_hotspot"
    private val main = Handler(Looper.getMainLooper())
    /** The running hotspot and the request it's for (main thread only). */
    private var running: Pair<String, WifiManager.LocalOnlyHotspotReservation>? = null

    /** This phone can start one (Android 8 or later, with Wi-Fi). */
    fun available(ctx: Context): Boolean =
        Build.VERSION.SDK_INT >= 26 && ctx.applicationContext.getSystemService(Context.WIFI_SERVICE) != null

    /** Start the hotspot for request [sid] (main thread). */
    @SuppressLint("MissingPermission")
    fun start(app: Context, sid: String) {
        if (Build.VERSION.SDK_INT < 26) return WifiDirect.hotspotFailed(sid, "needs Android 8")
        val wm = app.getSystemService(Context.WIFI_SERVICE) as? WifiManager
            ?: return WifiDirect.hotspotFailed(sid, "no Wi-Fi")
        // One at a time: a new request replaces the old one's hotspot.
        running?.second?.let { runCatching { it.close() } }
        running = null
        val staIp = runCatching {
            @Suppress("DEPRECATION")
            val ip = wm.connectionInfo?.ipAddress ?: 0
            if (ip == 0) null else "${ip and 0xff}.${ip shr 8 and 0xff}.${ip shr 16 and 0xff}.${ip shr 24 and 0xff}"
        }.getOrNull()
        val cb = object : WifiManager.LocalOnlyHotspotCallback() {
            override fun onStarted(reservation: WifiManager.LocalOnlyHotspotReservation) {
                running = sid to reservation
                ResourceLedger.hold(ResourceBook.Kind.HOTSPOT, "local-only hotspot")
                val (ssid, pass) = credentials(reservation)
                if (ssid.isNullOrEmpty() || pass.isNullOrEmpty()) {
                    runCatching { reservation.close() }
                    running = null
                    ResourceLedger.release(ResourceBook.Kind.HOTSPOT, "local-only hotspot")
                    return WifiDirect.hotspotFailed(sid, "the hotspot has no passphrase")
                }
                // Its address there, once the interface has one.
                main.postDelayed({ WifiDirect.hotspotUp(sid, ssid, pass, address(staIp)) }, 500)
                Log.i(TAG, "$sid: hotspot up")
            }

            override fun onStopped() {
                if (running?.first == sid) {
                    running = null
                    ResourceLedger.release(ResourceBook.Kind.HOTSPOT, "local-only hotspot")
                }
                Log.i(TAG, "$sid: hotspot stopped")
            }

            override fun onFailed(reason: Int) {
                WifiDirect.hotspotFailed(sid, "the hotspot didn't start ($reason)")
            }
        }
        runCatching { wm.startLocalOnlyHotspot(cb, main) }
            .onFailure { WifiDirect.hotspotFailed(sid, "the hotspot didn't start") }
    }

    /** Stop the hotspot for [sid], if it's the one running (main thread). */
    fun stop(sid: String) {
        val r = running ?: return
        if (r.first != sid) return
        running = null
        runCatching { r.second.close() }
        ResourceLedger.release(ResourceBook.Kind.HOTSPOT, "local-only hotspot")
        Log.i(TAG, "$sid: hotspot closed")
    }

    @Suppress("DEPRECATION")
    private fun credentials(r: WifiManager.LocalOnlyHotspotReservation): Pair<String?, String?> =
        if (Build.VERSION.SDK_INT >= 30) {
            val c = r.softApConfiguration
            c.ssid?.removeSurrounding("\"") to c.passphrase
        } else {
            val c = r.wifiConfiguration
            c?.SSID?.removeSurrounding("\"") to c?.preSharedKey?.removeSurrounding("\"")
        }

    private fun address(staIp: String?): String? = runCatching {
        val list = NetworkInterface.getNetworkInterfaces()?.toList().orEmpty()
            .filter { it.isUp && !it.isLoopback }
            .flatMap { nif ->
                nif.inetAddresses.toList().filterIsInstance<Inet4Address>().map { nif.name to (it.hostAddress ?: "") }
            }
        WifiDirectPlan.pickHotspotAddress(list, staIp)
    }.getOrNull()
}
