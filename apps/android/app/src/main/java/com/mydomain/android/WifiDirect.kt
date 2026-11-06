package com.mydomain.android

import android.annotation.SuppressLint
import android.app.Activity
import android.app.AlertDialog
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.net.wifi.p2p.WifiP2pConfig
import android.net.wifi.p2p.WifiP2pManager
import android.os.Build
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.util.Log
import android.widget.Toast
import androidx.core.app.NotificationCompat
import com.mydomain.android.core.Core
import com.mydomain.android.core.WifiDirectPlan
import com.mydomain.android.core.WifiDirectPlan.Event
import com.mydomain.android.core.WifiDirectPlan.Phase
import org.json.JSONObject

/**
 * Wi-Fi Direct for big files (mdcore `features::wifi_direct`): the core asks
 * this phone to host a group, join another device's (by its name and
 * passphrase, Android 10 or later), or leave it; once the group is up the
 * core opens its own encrypted link over it and the transfer moves there.
 * Anything that goes wrong is told to the core, and the transfer just
 * keeps its usual route.
 *
 * The same for this phone's local-only hotspot (method "hotspot",
 * [PhoneHotspot]): a computer without Wi-Fi Direct (a Mac, Linux) joins it
 * for the transfer. Both work with no internet and no server: the offer
 * can come over Bluetooth.
 *
 * Framework only (`WifiP2pManager`, `WifiManager`), no extra library. It needs location
 * (or "nearby Wi-Fi devices" for an app targeting Android 13): explained
 * and asked for the first time it's needed ([WifiDirectPermissionActivity]),
 * never again after a no. "Use Wi-Fi Direct for big files when nearby" in
 * Settings, on by default ("md" preference `wifi_direct`, in the settings
 * backup).
 */
object WifiDirect {
    private const val TAG = "mydomain_wifidirect"
    const val PREF = "wifi_direct"
    private const val PREF_ASKED = "wifi_direct_asked"
    private const val CH = "wifi_direct"
    const val NOTE_ASK = 7321
    /** How often, and how many times, the group's state is looked at. */
    private const val POLL_MS = 500L
    private const val POLLS = (WifiDirectPlan.SETUP_MS / POLL_MS).toInt()

    private val main = Handler(Looper.getMainLooper())
    private var manager: WifiP2pManager? = null
    private var channel: WifiP2pManager.Channel? = null
    /** sid -> where that request is (main thread only). */
    private val phases = HashMap<String, Phase>()

    private fun prefs(ctx: Context) = ctx.getSharedPreferences("md", Context.MODE_PRIVATE)

    fun enabled(ctx: Context): Boolean = prefs(ctx).getBoolean(PREF, true)

    fun setEnabled(ctx: Context, on: Boolean) {
        prefs(ctx).edit().putBoolean(PREF, on).apply()
        Thread { runCatching { Core.engine()?.setWifiDirectEnabled(on) } }.start()
    }

    /** This phone has Wi-Fi Direct at all. */
    fun hasFeature(ctx: Context): Boolean =
        ctx.packageManager.hasSystemFeature(PackageManager.FEATURE_WIFI_DIRECT)

    /** Tell the core what this phone can do and the user's switch (once the
     *  engine is up). */
    fun init(ctx: Context) {
        val e = Core.engine() ?: return
        runCatching {
            e.setWifiDirectEnabled(enabled(ctx))
            e.setWifiDirectCaps(WifiDirectPlan.caps(Build.VERSION.SDK_INT, hasFeature(ctx), PhoneHotspot.available(ctx)).toULong())
        }.onFailure { Log.w(TAG, "can't tell the core: $it") }
    }

    fun permissions(): List<String> =
        WifiDirectPlan.permissionsFor(Build.VERSION.SDK_INT, BuildConfig.TARGET_SDK)

    fun permitted(ctx: Context): Boolean =
        permissions().all { ctx.checkSelfPermission(it) == PackageManager.PERMISSION_GRANTED }

    /** A `wifi_direct_request` event from the core (any thread). */
    fun onRequest(ctx: Context, payloadJson: String) {
        val o = runCatching { JSONObject(payloadJson) }.getOrNull() ?: return
        val app = ctx.applicationContext
        val sid = o.optString("sid")
        val op = o.optString("op")
        val method = o.optString("method", "wifi_direct")
        val ssid = o.optString("ssid")
        val pass = o.optString("passphrase")
        if (sid.isEmpty()) return
        main.post {
            if (method == "hotspot") {
                when (op) {
                    "host" -> start(app, sid, Event.Host, needsP2p = false) { PhoneHotspot.start(app, sid) }
                    "leave" -> { phases.remove(sid); PhoneHotspot.stop(sid) }
                    // A phone never leaves its own Wi-Fi for another's hotspot.
                    else -> fail(sid, "a phone only hosts a hotspot")
                }
                return@post
            }
            when (op) {
                "host" -> start(app, sid, Event.Host) { host(app, sid, ssid, pass) }
                "join" -> start(app, sid, Event.Join) { join(app, sid, ssid, pass) }
                "leave" -> leave(sid)
            }
        }
    }

    private fun start(app: Context, sid: String, event: Event, needsP2p: Boolean = true, go: () -> Unit) {
        val next = WifiDirectPlan.next(phases[sid] ?: Phase.Idle, event) ?: return
        if (!enabled(app) || (needsP2p && !hasFeature(app))) return fail(sid, "off")
        if (!permitted(app)) {
            askOnce(app)
            return fail(sid, "no_permission")
        }
        if (!needsP2p) {
            phases[sid] = next
            return go()
        }
        val m = manager ?: (app.getSystemService(Context.WIFI_P2P_SERVICE) as? WifiP2pManager)?.also { manager = it }
        if (m == null) return fail(sid, "no Wi-Fi Direct")
        if (channel == null) channel = runCatching { m.initialize(app, Looper.getMainLooper(), null) }.getOrNull()
        if (channel == null) return fail(sid, "no channel")
        phases[sid] = next
        go()
    }

    /** The hotspot for [sid] is up (main thread). */
    fun hotspotUp(sid: String, ssid: String, pass: String, hostIp: String?) {
        if (!move(sid, Event.GroupUp)) return
        Thread { runCatching { Core.engine()?.wifiDirectHosted(sid, ssid, pass, hostIp.orEmpty()) } }.start()
    }

    /** The hotspot for [sid] couldn't start (main thread). */
    fun hotspotFailed(sid: String, why: String) = fail(sid, why)

    private fun move(sid: String, event: Event): Boolean {
        val next = WifiDirectPlan.next(phases[sid] ?: return false, event) ?: return false
        phases[sid] = next
        return true
    }

    private fun fail(sid: String, why: String) {
        Log.i(TAG, "$sid: $why")
        if (phases.containsKey(sid)) move(sid, Event.Failure)
        Thread { runCatching { Core.engine()?.wifiDirectFailed(sid, why) } }.start()
    }

    private fun listener(sid: String, what: String, ok: () -> Unit) = object : WifiP2pManager.ActionListener {
        override fun onSuccess() = ok()
        override fun onFailure(reason: Int) = fail(sid, "$what failed ($reason)")
    }

    @SuppressLint("MissingPermission")
    private fun host(app: Context, sid: String, ssid: String, pass: String) {
        val m = manager ?: return
        val ch = channel ?: return
        val whenUp = listener(sid, "createGroup") { waitForGroup(sid, POLLS) }
        if (Build.VERSION.SDK_INT >= 29 && WifiDirectPlan.validGroup(ssid, pass)) {
            val config = WifiP2pConfig.Builder()
                .setNetworkName(ssid)
                .setPassphrase(pass)
                .enablePersistentMode(false)
                .build()
            m.createGroup(ch, config, whenUp)
        } else {
            m.createGroup(ch, whenUp)
        }
        Log.i(TAG, "$sid: hosting (${app.packageName})")
    }

    /** The group is made: its name and passphrase go to the core. */
    @SuppressLint("MissingPermission")
    private fun waitForGroup(sid: String, left: Int) {
        val m = manager ?: return
        val ch = channel ?: return
        if (phases[sid] != Phase.Hosting) return
        m.requestGroupInfo(ch) { group ->
            val name = group?.networkName.orEmpty()
            val pass = group?.passphrase.orEmpty()
            when {
                group != null && group.isGroupOwner && name.isNotEmpty() && pass.isNotEmpty() -> {
                    if (move(sid, Event.GroupUp)) {
                        Thread { runCatching { Core.engine()?.wifiDirectHosted(sid, name, pass, "") } }.start()
                    }
                }
                left > 0 -> main.postDelayed({ waitForGroup(sid, left - 1) }, POLL_MS)
                else -> fail(sid, "the group didn't come up")
            }
        }
    }

    @SuppressLint("MissingPermission")
    private fun join(app: Context, sid: String, ssid: String, pass: String) {
        val m = manager ?: return
        val ch = channel ?: return
        if (Build.VERSION.SDK_INT < 29 || !WifiDirectPlan.validGroup(ssid, pass)) {
            return fail(sid, "can't join by name before Android 10")
        }
        val config = WifiP2pConfig.Builder()
            .setNetworkName(ssid)
            .setPassphrase(pass)
            .enablePersistentMode(false)
            .build()
        m.connect(ch, config, listener(sid, "connect") { waitForOwner(sid, POLLS) })
        Log.i(TAG, "$sid: joining (${app.packageName})")
    }

    /** Joined: the group owner's address goes to the core, which connects. */
    private fun waitForOwner(sid: String, left: Int) {
        val m = manager ?: return
        val ch = channel ?: return
        if (phases[sid] != Phase.Joining) return
        m.requestConnectionInfo(ch) { info ->
            val owner = info?.groupOwnerAddress?.hostAddress
            when {
                info != null && info.groupFormed && !info.isGroupOwner && !owner.isNullOrEmpty() -> {
                    if (move(sid, Event.Connected)) {
                        Thread { runCatching { Core.engine()?.wifiDirectJoined(sid, owner) } }.start()
                    }
                }
                left > 0 -> main.postDelayed({ waitForOwner(sid, left - 1) }, POLL_MS)
                else -> fail(sid, "didn't join the group")
            }
        }
    }

    /** The core is done with it: the group goes. */
    private fun leave(sid: String) {
        val was = phases[sid] ?: Phase.Idle
        phases.remove(sid)
        if (!WifiDirectPlan.removeOnLeave(was)) return
        val m = manager ?: return
        val ch = channel ?: return
        runCatching { m.removeGroup(ch, null) }
        if (was == Phase.Joining) runCatching { m.cancelConnect(ch, null) }
        Log.i(TAG, "$sid: left")
    }

    /** The first time it's needed: say why and ask (on screen when the app
     *  is, else by a notification). Never again after that. */
    private fun askOnce(app: Context) {
        if (!WifiDirectPlan.shouldAsk(permitted(app), prefs(app).getBoolean(PREF_ASKED, false))) return
        prefs(app).edit().putBoolean(PREF_ASKED, true).apply()
        val ask = Intent(app, WifiDirectPermissionActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
        if (Core.visible && runCatching { app.startActivity(ask) }.isSuccess) return
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            app.getSystemService(NotificationManager::class.java)?.createNotificationChannel(
                NotificationChannel(CH, app.getString(R.string.channel_wifi_direct), NotificationManager.IMPORTANCE_DEFAULT),
            )
        }
        val pi = PendingIntent.getActivity(app, NOTE_ASK, ask, PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
        val n = NotificationCompat.Builder(app, CH)
            .setSmallIcon(android.R.drawable.stat_sys_upload)
            .setContentTitle(app.getString(R.string.wifi_direct_ask_title))
            .setContentText(app.getString(R.string.wifi_direct_ask_short))
            .setAutoCancel(true)
            .setContentIntent(pi)
            .build()
        runCatching { app.getSystemService(NotificationManager::class.java)?.notify(NOTE_ASK, n) }
    }
}

/** "Use Wi-Fi Direct for big files?": why it needs the permission, then
 *  Android's own prompt. A no is fine: big files keep their usual route. */
class WifiDirectPermissionActivity : Activity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        getSystemService(NotificationManager::class.java)?.cancel(WifiDirect.NOTE_ASK)
        if (WifiDirect.permitted(this)) return finish()
        AlertDialog.Builder(this, android.R.style.Theme_DeviceDefault_Dialog_Alert)
            .setTitle(R.string.wifi_direct_ask_title)
            .setMessage(R.string.wifi_direct_ask_body)
            .setPositiveButton(R.string.wifi_direct_allow) { _, _ ->
                requestPermissions(WifiDirect.permissions().toTypedArray(), ASK)
            }
            .setNegativeButton(R.string.wifi_direct_not_now) { _, _ -> finish() }
            .setOnCancelListener { finish() }
            .show()
    }

    override fun onRequestPermissionsResult(requestCode: Int, permissions: Array<out String>, grantResults: IntArray) {
        super.onRequestPermissionsResult(requestCode, permissions, grantResults)
        if (requestCode != ASK) return
        val ok = grantResults.isNotEmpty() && grantResults.all { it == PackageManager.PERMISSION_GRANTED }
        Toast.makeText(this, getString(if (ok) R.string.wifi_direct_allowed else R.string.wifi_direct_denied), Toast.LENGTH_LONG).show()
        finish()
    }

    private companion object {
        const val ASK = 1
    }
}
