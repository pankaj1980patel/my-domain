package com.mydomain.android.core

/**
 * Staying reachable, the pure part (no Android types, unit-tested): which
 * network changes the core is told about, what kind of network it is, and
 * when the service checks it's still running.
 *
 * Android reports the default network through a callback that fires a lot
 * (onAvailable, onLost, and onCapabilitiesChanged for every signal-strength
 * step). The service turns each burst into one [Snapshot] after
 * [DEBOUNCE_MS] and tells the core only when it [significant]ly changed: a
 * new network (a Wi-Fi switch too), the transport, internet validated or
 * not, or the Wi-Fi address. The core then redials at once and backs off
 * (`features::reconnect`).
 */
object NetWatch {
    /** One burst of callbacks becomes one look after this long. */
    const val DEBOUNCE_MS = 400L

    /** The health check runs about this often while all is well ... */
    const val CHECK_EVERY_MS = 15 * 60 * 1000L

    /** ... and sooner after it found the service gone (killed by the
     *  phone's battery manager): this, doubling up to [CHECK_EVERY_MS]. */
    const val CHECK_AGAIN_MS = 60 * 1000L

    /** The default network as the service sees it. [network] null: none. */
    data class Snapshot(
        val network: Long?,
        val wifi: Boolean = false,
        val cellular: Boolean = false,
        val ethernet: Boolean = false,
        val validated: Boolean = false,
        val wifiIp: String = "",
    )

    /** What the core is told (`Engine::network_changed_on`). */
    fun kind(s: Snapshot?): String = when {
        s?.network == null -> "offline"
        s.wifi -> "wifi"
        s.ethernet -> "ethernet"
        s.cellular -> "mobile"
        else -> "unknown"
    }

    /** Whether going from [prev] to [next] is worth telling the core. */
    fun significant(prev: Snapshot?, next: Snapshot?): Boolean = prev != next

    /**
     * `base` doubled [attempt] times, at most [cap], spread ±20% by
     * [jitter] (0..1): the same schedule as the core's redial
     * (`reconnect::backoff`).
     */
    fun backoffMs(attempt: Int, base: Long, cap: Long, jitter: Double): Long {
        val factor = 1L shl attempt.coerceIn(0, 16)
        val exp = (base * factor).coerceAtMost(cap)
        return (exp * (0.8 + 0.4 * jitter.coerceIn(0.0, 1.0))).toLong()
    }

    /** When the next health check runs: the usual [CHECK_EVERY_MS], or
     *  sooner after [deadInARow] checks found the service gone. */
    fun nextCheckMs(deadInARow: Int, jitter: Double): Long =
        if (deadInARow <= 0) CHECK_EVERY_MS
        else backoffMs(deadInARow - 1, CHECK_AGAIN_MS, CHECK_EVERY_MS, jitter)

    /** Does that check wake the phone? Not while all is well: a service
     *  gone while the phone sleeps can wait for its next wake (a push
     *  starts it anyway), so the usual check rides along then. Only the
     *  quicker checks after it was killed wake the phone. */
    fun checkWakes(deadInARow: Int): Boolean = deadInARow > 0
}

/**
 * The phone maker's own "autostart" / background-activity page, where
 * my-domain must be allowed to start by itself and run in the background,
 * or the maker's battery manager stops it no matter what Android allows.
 * Pure (unit-tested); [com.mydomain.android.SetupState] tries each page in
 * order and falls back to this app's Settings page.
 */
object OemAutostart {
    data class Page(val pkg: String, val cls: String)

    enum class Maker { XIAOMI, OPPO, ONEPLUS, VIVO, SAMSUNG, HUAWEI }

    /** Which maker's pages apply, from `Build.MANUFACTURER` and `Build.BRAND`. */
    fun maker(manufacturer: String, brand: String = ""): Maker? {
        val m = manufacturer.trim().lowercase()
        val b = brand.trim().lowercase()
        fun any(vararg names: String) = names.any { m == it || b == it }
        return when {
            any("xiaomi", "redmi", "poco") -> Maker.XIAOMI
            any("oneplus") -> Maker.ONEPLUS
            any("oppo", "realme") -> Maker.OPPO
            any("vivo", "iqoo") -> Maker.VIVO
            any("samsung") -> Maker.SAMSUNG
            any("huawei", "honor") -> Maker.HUAWEI
            else -> null
        }
    }

    private val COLOROS = listOf(
        Page("com.coloros.safecenter", "com.coloros.safecenter.permission.startup.StartupAppListActivity"),
        Page("com.coloros.safecenter", "com.coloros.safecenter.startupapp.StartupAppListActivity"),
        Page("com.oppo.safe", "com.oppo.safe.permission.startup.StartupAppListActivity"),
        Page("com.coloros.oppoguardelf", "com.coloros.powermanager.fuelgaue.PowerUsageModelActivity"),
    )

    /** The pages to try, most specific first; empty: none known. */
    fun pages(manufacturer: String, brand: String = ""): List<Page> = when (maker(manufacturer, brand)) {
        Maker.XIAOMI -> listOf(
            Page("com.miui.securitycenter", "com.miui.permcenter.autostart.AutoStartManagementActivity"),
            Page("com.miui.powerkeeper", "com.miui.powerkeeper.ui.HiddenAppsConfigActivity"),
        )
        Maker.OPPO -> COLOROS
        // Newer OnePlus phones run ColorOS.
        Maker.ONEPLUS -> listOf(
            Page("com.oneplus.security", "com.oneplus.security.chainlaunch.view.ChainLaunchAppListActivity"),
        ) + COLOROS
        // vivo and iQOO (Funtouch OS / OriginOS): background start-up, then
        // "high background power consumption".
        Maker.VIVO -> listOf(
            Page("com.vivo.permissionmanager", "com.vivo.permissionmanager.activity.BgStartUpManagerActivity"),
            Page("com.iqoo.secure", "com.iqoo.secure.ui.phoneoptimize.BgStartUpManager"),
            Page("com.iqoo.secure", "com.iqoo.secure.ui.phoneoptimize.AddWhiteListActivity"),
            Page("com.vivo.abe", "com.vivo.applicationbehaviorengine.ui.ExcessivePowerManagerActivity"),
        )
        // Battery > Background usage limits ("Never sleeping apps").
        Maker.SAMSUNG -> listOf(
            Page("com.samsung.android.lool", "com.samsung.android.sm.battery.ui.BatteryActivity"),
            Page("com.samsung.android.lool", "com.samsung.android.sm.ui.battery.BatteryActivity"),
            Page("com.samsung.android.sm", "com.samsung.android.sm.ui.battery.BatteryActivity"),
        )
        Maker.HUAWEI -> listOf(
            Page("com.huawei.systemmanager", "com.huawei.systemmanager.startupmgr.ui.StartupNormalAppListActivity"),
            Page("com.huawei.systemmanager", "com.huawei.systemmanager.optimize.process.ProtectActivity"),
        )
        null -> emptyList()
    }

    /** This phone has a maker's page to show. */
    fun known(manufacturer: String, brand: String = ""): Boolean = maker(manufacturer, brand) != null
}
