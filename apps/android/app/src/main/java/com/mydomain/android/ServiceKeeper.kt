package com.mydomain.android

import android.app.AlarmManager
import android.app.PendingIntent
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.os.Build
import android.os.SystemClock
import android.util.Log
import com.mydomain.android.core.Core
import com.mydomain.android.core.NetWatch

/**
 * Keeps NetService (and with it the link to your computers) running:
 *
 * - started again after the phone restarts and after the app is updated
 *   ([ServiceKeeperReceiver] on BOOT_COMPLETED / MY_PACKAGE_REPLACED), once
 *   it has run at least once (the app was opened);
 * - a health check every [NetWatch.CHECK_EVERY_MS] or so (an inexact alarm
 *   that may run in Doze: no exact-alarm permission, no WorkManager), which
 *   starts the service if the phone killed it, and comes back sooner while
 *   that keeps happening ([NetWatch.nextCheckMs]).
 *
 * The service itself is sticky (Android restarts it after killing it), and
 * a push starts it too (MdMessagingService); this is for the phones whose
 * battery manager stops apps for good. Letting my-domain run in the
 * background (Set up this phone: battery, autostart) matters more.
 */
object ServiceKeeper {
    const val ACTION_CHECK = "com.mydomain.android.action.SERVICE_CHECK"
    private const val PREFS = "md"
    /** The service ran at least once: bring it back after a restart. */
    private const val KEY_ON = "keeper_on"
    /** Checks in a row that found it gone. */
    private const val KEY_DEAD = "keeper_dead"
    private const val TAG = "mydomain_keeper"

    /** The service is up: all is well, next check in the usual time. */
    fun serviceUp(ctx: Context) {
        val p = prefs(ctx)
        if (!p.getBoolean(KEY_ON, false) || p.getInt(KEY_DEAD, 0) != 0) {
            p.edit().putBoolean(KEY_ON, true).putInt(KEY_DEAD, 0).apply()
        }
        schedule(ctx, NetWatch.nextCheckMs(0, Math.random()), NetWatch.checkWakes(0))
    }

    /** Check in a few seconds (the app was swiped away). */
    fun checkSoon(ctx: Context) = schedule(ctx, 5_000, wake = true)

    /** A check, a restart or an update: start the service if it's gone. */
    fun check(ctx: Context, why: String) {
        val p = prefs(ctx)
        if (!p.getBoolean(KEY_ON, false)) return
        if (NetService.running) {
            serviceUp(ctx)
            return
        }
        val dead = p.getInt(KEY_DEAD, 0) + 1
        p.edit().putInt(KEY_DEAD, dead).apply()
        Log.i(TAG, "service not running ($why): starting it")
        runCatching { Core.labLog("keeper: restart ($why)") }
        val svc = Intent(ctx, NetService::class.java)
        runCatching {
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) ctx.startForegroundService(svc) else ctx.startService(svc)
        }.onFailure { Log.w(TAG, "can't start the service: $it") }
        // Sooner while the phone keeps killing it; the service's own start
        // puts it back to the usual time.
        schedule(ctx, NetWatch.nextCheckMs(dead, Math.random()), NetWatch.checkWakes(dead))
    }

    /** [wake]: an alarm that wakes the phone (and may run in Doze); else
     *  one that waits for the phone's next wake. */
    private fun schedule(ctx: Context, delayMs: Long, wake: Boolean) {
        val am = ctx.getSystemService(AlarmManager::class.java) ?: return
        val pi = PendingIntent.getBroadcast(
            ctx, 0,
            Intent(ctx, ServiceKeeperReceiver::class.java).setAction(ACTION_CHECK),
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
        )
        val at = SystemClock.elapsedRealtime() + delayMs
        runCatching {
            if (wake) am.setAndAllowWhileIdle(AlarmManager.ELAPSED_REALTIME_WAKEUP, at, pi)
            else am.set(AlarmManager.ELAPSED_REALTIME, at, pi)
        }.onFailure { Log.w(TAG, "can't schedule the check: $it") }
    }

    private fun prefs(ctx: Context) = ctx.applicationContext.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
}

/** The health check's alarm, the phone's restart and the app's update. */
class ServiceKeeperReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        when (intent.action) {
            Intent.ACTION_BOOT_COMPLETED -> ServiceKeeper.check(context.applicationContext, "boot")
            Intent.ACTION_MY_PACKAGE_REPLACED -> ServiceKeeper.check(context.applicationContext, "update")
            ServiceKeeper.ACTION_CHECK -> ServiceKeeper.check(context.applicationContext, "check")
        }
    }
}
