package com.mydomain.android

import android.Manifest
import android.annotation.SuppressLint
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.location.Location
import android.location.LocationManager
import android.os.Build
import android.provider.Settings
import android.util.Log
import androidx.core.app.NotificationCompat
import com.mydomain.android.core.Core
import org.json.JSONObject
import java.util.concurrent.Executors
import com.mydomain.android.core.ResourceBook
import com.mydomain.android.core.ResourceLedger

/**
 * Where this phone is, for your own computers (find my phone): the
 * `locate` action puts `location {lat, lon, acc, at}` in the status.
 *
 * The phone stays in charge, so this can't become quiet tracking:
 *  - off until you turn on "Let my computers see where this phone is" in
 *    my-domain's Settings ([PREF]); asked while off, the phone opens that
 *    setting (or notifies), and shares nothing;
 *  - every time it's shared, a notification on the phone says who asked;
 *  - Android's own location permission and Location switch still apply
 *    (the phone opens the prompt / settings when one is missing).
 * Only on request: nothing is sent in the background.
 */
object PhoneLocation {
    private const val TAG = "mydomain_location"
    const val PREF = "share_location"
    private const val NOTE_ASK = 7601
    private const val NOTE_SHARED = 7602
    /** A fresh fix gets this long before the last known one is used. */
    private const val FIX_TIMEOUT_MS = 20_000L

    @Volatile
    private var last: JSONObject? = null

    fun allowed(ctx: Context): Boolean =
        ctx.getSharedPreferences("md", Context.MODE_PRIVATE).getBoolean(PREF, false)

    fun setAllowed(ctx: Context, on: Boolean) {
        ctx.getSharedPreferences("md", Context.MODE_PRIVATE).edit().putBoolean(PREF, on).apply()
        if (!on) last = null
        Controls.onChange()
    }

    /** The last shared location, into the status (only while allowed). */
    fun state(ctx: Context, o: JSONObject) {
        if (!allowed(ctx)) return
        o.put("location_sharing", true)
        last?.let { o.put("location", it) }
    }

    /** Another device asked where this phone is. */
    fun locate(ctx: Context, fromName: String): Boolean {
        val app = ctx.applicationContext
        if (!allowed(app)) {
            ask(app, Intent(app, MainActivity::class.java).putExtra(EXTRA_OPEN_SETTINGS, true), app.getString(R.string.location_ask_allow, fromName))
            return false
        }
        if (app.checkSelfPermission(Manifest.permission.ACCESS_FINE_LOCATION) != PackageManager.PERMISSION_GRANTED) {
            ask(app, Intent(app, MainActivity::class.java).putExtra(WifiNetworks.EXTRA_ASK, true), app.getString(R.string.location_ask_permission))
            return false
        }
        val lm = app.getSystemService(LocationManager::class.java) ?: return false
        val on = Build.VERSION.SDK_INT < Build.VERSION_CODES.P || lm.isLocationEnabled
        if (!on) {
            ask(app, Intent(Settings.ACTION_LOCATION_SOURCE_SETTINGS), app.getString(R.string.location_ask_on))
            return false
        }
        fix(app, lm, fromName)
        return true
    }

    /** Like [locate], but only when sharing, the permission and Location are
     *  all already on: it never opens a screen or asks (lost mode, with the
     *  phone locked). Off: nothing is shared. */
    fun locateQuietly(ctx: Context, fromName: String): Boolean {
        val app = ctx.applicationContext
        if (!allowed(app)) return false
        if (app.checkSelfPermission(Manifest.permission.ACCESS_FINE_LOCATION) != PackageManager.PERMISSION_GRANTED) return false
        val lm = app.getSystemService(LocationManager::class.java) ?: return false
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.P && !lm.isLocationEnabled) return false
        fix(app, lm, fromName)
        return true
    }

    /** The last known position at once, then a fresh one when it comes. */
    @SuppressLint("MissingPermission")
    private fun fix(app: Context, lm: LocationManager, fromName: String) {
        val providers = buildList {
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) add(LocationManager.FUSED_PROVIDER)
            add(LocationManager.GPS_PROVIDER)
            add(LocationManager.NETWORK_PROVIDER)
        }.filter { runCatching { lm.isProviderEnabled(it) }.getOrDefault(false) }
        providers.mapNotNull { runCatching { lm.getLastKnownLocation(it) }.getOrNull() }
            .maxByOrNull { it.time }
            ?.let { share(app, it, fromName) }
        val best = providers.firstOrNull() ?: return
        ResourceLedger.hold(ResourceBook.Kind.LOCATION, "fix ($best)", FIX_TIMEOUT_MS)
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
            // Android gives up by itself (about 30 s); the cancel is a backstop.
            val cancel = android.os.CancellationSignal()
            main.postDelayed({ cancel.cancel() }, FIX_TIMEOUT_MS)
            runCatching {
                lm.getCurrentLocation(best, cancel, fixer) { loc ->
                    ResourceLedger.release(ResourceBook.Kind.LOCATION, "fix ($best)")
                    loc?.let { share(app, it, fromName) }
                }
            }.onFailure { Log.w(TAG, "no fix: $it") }
        } else {
            // A single update has no timeout: indoors the GPS would look
            // for ever. Stop looking after FIX_TIMEOUT_MS.
            val listener = object : android.location.LocationListener {
                override fun onLocationChanged(loc: Location) {
                    main.removeCallbacksAndMessages(this)
                    ResourceLedger.release(ResourceBook.Kind.LOCATION, "fix ($best)")
                    share(app, loc, fromName)
                }

                @Deprecated("Deprecated in Java")
                override fun onStatusChanged(provider: String?, status: Int, extras: android.os.Bundle?) {}
                override fun onProviderEnabled(provider: String) {}
                override fun onProviderDisabled(provider: String) {}
            }
            @Suppress("DEPRECATION")
            runCatching {
                lm.requestSingleUpdate(best, listener, android.os.Looper.getMainLooper())
                main.postAtTime({
                    runCatching { lm.removeUpdates(listener) }
                    ResourceLedger.release(ResourceBook.Kind.LOCATION, "fix ($best)")
                }, listener, android.os.SystemClock.uptimeMillis() + FIX_TIMEOUT_MS)
            }
        }
    }

    private val main = android.os.Handler(android.os.Looper.getMainLooper())
    /** One thread for the fixes' answers, not a new one each time. */
    private val fixer by lazy { Executors.newSingleThreadExecutor() }

    private fun share(app: Context, loc: Location, fromName: String) {
        if (!allowed(app)) return
        last = JSONObject()
            .put("lat", loc.latitude)
            .put("lon", loc.longitude)
            .put("acc", loc.accuracy.toDouble())
            .put("at", loc.time / 1000)
        Log.i(TAG, "shared with $fromName (±${loc.accuracy.toInt()} m)")
        Controls.onChange()
        notifyShared(app, fromName)
    }

    /** Every share is visible on the phone. */
    private fun notifyShared(app: Context, fromName: String) {
        val pi = PendingIntent.getActivity(
            app, NOTE_SHARED,
            Intent(app, MainActivity::class.java).putExtra(EXTRA_OPEN_SETTINGS, true).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
        val n = NotificationCompat.Builder(app, DeviceActions.CH_LINKS)
            .setSmallIcon(android.R.drawable.ic_menu_mylocation)
            .setContentTitle(app.getString(R.string.location_shared_title, fromName))
            .setContentText(app.getString(R.string.location_shared_body))
            .setPriority(NotificationCompat.PRIORITY_LOW)
            .setAutoCancel(true)
            .setContentIntent(pi)
            .build()
        runCatching { app.getSystemService(NotificationManager::class.java)?.notify(NOTE_SHARED, n) }
    }

    /** Open [open] straight away when Android lets this app open a screen,
     *  else a notification saying [text] that opens it. */
    private fun ask(app: Context, open: Intent, text: String) {
        open.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
        if ((Core.visible || PhoneHelper.instance != null) && runCatching { app.startActivity(open) }.isSuccess) return
        val pi = PendingIntent.getActivity(app, NOTE_ASK, open, PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
        val n = NotificationCompat.Builder(app, DeviceActions.CH_LINKS)
            .setSmallIcon(android.R.drawable.ic_menu_mylocation)
            .setContentTitle(app.getString(R.string.location_ask_title))
            .setContentText(text)
            .setStyle(NotificationCompat.BigTextStyle().bigText(text))
            .setAutoCancel(true)
            .setContentIntent(pi)
            .build()
        runCatching { app.getSystemService(NotificationManager::class.java)?.notify(NOTE_ASK, n) }
    }

    /** MainActivity: open the Settings screen (where the switch is). */
    const val EXTRA_OPEN_SETTINGS = "open_settings"
}
