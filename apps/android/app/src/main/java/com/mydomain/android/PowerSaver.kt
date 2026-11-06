package com.mydomain.android

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.net.ConnectivityManager
import android.net.Network
import android.net.NetworkCapabilities
import android.os.Handler
import android.os.Looper
import android.os.PowerManager
import android.util.Log
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import com.mydomain.android.core.Core
import com.mydomain.android.core.PowerPolicy

/**
 * Save battery, the phone's part: watches Battery Saver, the battery level,
 * Doze and whether the network is metered, decides with [PowerPolicy] what
 * heavy background work waits, and applies it: the core's switch
 * (`set_power_policy`: synced-folder rounds, the photo backup, the size cap
 * for synced files on a metered network), the photo backup's network rule,
 * and `saving` in this phone's status (the computer shows "Saving battery").
 * Runs inside [NetService]; the settings live in the "md" preferences.
 */
object PowerSaver {
    private const val TAG = "mydomain_power"
    const val PREF_MODE = "save_battery_mode"
    const val PREF_THRESHOLD = "save_battery_threshold"
    const val PREF_MOBILE_DATA = "save_battery_mobile_data"

    private val main = Handler(Looper.getMainLooper())
    private var app: Context? = null
    private var receiver: BroadcastReceiver? = null
    private var netCallback: ConnectivityManager.NetworkCallback? = null
    private var onChange: (() -> Unit)? = null

    @Volatile private var level: Int? = null
    @Volatile private var charging = false

    /** The plan in force (any thread). */
    @Volatile var current: PowerPolicy.Plan = PowerPolicy.Plan.NONE
        private set

    /** The same, for the screens (main thread). */
    var plan by mutableStateOf(PowerPolicy.Plan.NONE)
        private set

    /** The settings, for the screens (main thread). */
    var settings by mutableStateOf(PowerPolicy.Settings())
        private set

    /** The core heard a plan since the start. */
    @Volatile private var applied = false

    private fun prefs(ctx: Context) = ctx.getSharedPreferences("md", Context.MODE_PRIVATE)

    fun load(ctx: Context): PowerPolicy.Settings {
        val p = prefs(ctx)
        return PowerPolicy.Settings(
            mode = PowerPolicy.Mode.of(p.getString(PREF_MODE, null)),
            threshold = PowerPolicy.clampThreshold(p.getInt(PREF_THRESHOLD, PowerPolicy.DEFAULT_THRESHOLD)),
            mobileData = p.getBoolean(PREF_MOBILE_DATA, false),
        )
    }

    /** Start watching (from NetService, once the engine runs); [changed]
     *  is called whenever the plan changes (to push the status). */
    fun start(ctx: Context, changed: () -> Unit) {
        val c = ctx.applicationContext
        app = c
        onChange = changed
        applied = false
        settings = load(c)
        val r = object : BroadcastReceiver() {
            override fun onReceive(context: Context, intent: Intent) = update()
        }
        receiver = r
        val filter = IntentFilter().apply {
            addAction(PowerManager.ACTION_POWER_SAVE_MODE_CHANGED)
            addAction(PowerManager.ACTION_DEVICE_IDLE_MODE_CHANGED)
        }
        runCatching { c.registerReceiver(r, filter) }.onFailure { Log.w(TAG, "power watch failed", it) }
        val cb = object : ConnectivityManager.NetworkCallback() {
            override fun onAvailable(network: Network) = update()
            override fun onLost(network: Network) = update()
            override fun onCapabilitiesChanged(network: Network, caps: NetworkCapabilities) = update()
        }
        netCallback = cb
        runCatching { c.getSystemService(ConnectivityManager::class.java)?.registerDefaultNetworkCallback(cb) }
        update()
    }

    fun stop() {
        val c = app ?: return
        receiver?.let { runCatching { c.unregisterReceiver(it) } }
        netCallback?.let { runCatching { c.getSystemService(ConnectivityManager::class.java)?.unregisterNetworkCallback(it) } }
        receiver = null
        netCallback = null
        onChange = null
    }

    /** The battery changed (from NetService's battery watch). */
    fun onBattery(pct: Int, isCharging: Boolean) {
        if (level == pct && charging == isCharging) return
        level = pct
        charging = isCharging
        update()
    }

    /** The photo backup may send over the current network (asked afresh:
     *  the backup's own network callback may run before ours). */
    fun backupNetworkOk(): Boolean {
        val c = app ?: return false
        val cm = c.getSystemService(ConnectivityManager::class.java) ?: return false
        if (runCatching { cm.activeNetwork }.getOrNull() == null) return false
        return PowerPolicy.decide(load(c), conditions(c)).backupNetworkOk
    }

    // --- settings (main thread) ---------------------------------------------------

    /** Show the stored settings (the screen may open before the service). */
    fun refresh(ctx: Context) {
        settings = load(ctx)
    }

    fun setMode(ctx: Context, mode: PowerPolicy.Mode) = save(ctx) { it.copy(mode = mode) }
    fun setThreshold(ctx: Context, pct: Int) = save(ctx) { it.copy(threshold = PowerPolicy.clampThreshold(pct)) }
    fun setMobileData(ctx: Context, on: Boolean) = save(ctx) { it.copy(mobileData = on) }

    private fun save(ctx: Context, change: (PowerPolicy.Settings) -> PowerPolicy.Settings) {
        val s = change(load(ctx))
        settings = s
        prefs(ctx).edit()
            .putString(PREF_MODE, s.mode.key)
            .putInt(PREF_THRESHOLD, s.threshold)
            .putBoolean(PREF_MOBILE_DATA, s.mobileData)
            .apply()
        if (app == null) app = ctx.applicationContext
        update()
    }

    // --- deciding -----------------------------------------------------------------

    private fun conditions(c: Context): PowerPolicy.Conditions {
        val pm = c.getSystemService(PowerManager::class.java)
        val cm = c.getSystemService(ConnectivityManager::class.java)
        val net = runCatching { cm?.activeNetwork }.getOrNull()
        // No network: nothing goes anyway; not counted as metered.
        val metered = net != null && runCatching { cm?.isActiveNetworkMetered == true }.getOrDefault(false)
        return PowerPolicy.Conditions(
            powerSave = runCatching { pm?.isPowerSaveMode == true }.getOrDefault(false),
            level = level,
            charging = charging,
            idle = runCatching { pm?.isDeviceIdleMode == true }.getOrDefault(false),
            metered = metered,
        )
    }

    /** Decide again and apply what changed (any thread). */
    private fun update() {
        val c = app ?: return
        val s = load(c)
        val p = PowerPolicy.decide(s, conditions(c))
        val was = current
        current = p
        main.post {
            settings = s
            plan = p
        }
        // The network callback fires often (signal strength): the core and
        // the backup hear only a change (and the first plan).
        if (p == was && applied) return
        applied = true
        Log.i(TAG, "saving=${p.saving} reasons=${p.reasons} cap=${p.meteredCap}")
        Thread {
            runCatching { Core.engine()?.setPowerPolicy(p.heavyAllowed, p.meteredCap.toULong()) }
            PhotoBackup.pushConditions()
        }.start()
        if (p.saving != was.saving) onChange?.invoke()
    }
}
