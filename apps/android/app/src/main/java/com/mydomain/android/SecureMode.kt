package com.mydomain.android

import android.app.Activity
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.os.Build
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.provider.Settings
import android.service.quicksettings.Tile
import android.service.quicksettings.TileService
import android.util.Log
import android.widget.Toast
import androidx.core.app.NotificationCompat
import com.mydomain.android.core.SecureModePlan

/**
 * Secure mode: the phone helper ([PhoneHelper], my-domain's Accessibility
 * service) switched off, and back on. Banking and train-ticket apps refuse
 * to run, or warn, while any Accessibility service is on; the helper is
 * only needed for a computer's taps and swipes, phone keys pressed from a
 * computer and tapping radio switches in Settings.
 *
 *  - On: the helper switches itself off (`disableSelf`).
 *  - Off: Android lets no ordinary app turn an Accessibility service on.
 *    If you granted this app WRITE_SECURE_SETTINGS yourself over adb, it
 *    puts the helper back in Android's list at once; otherwise Android's
 *    Accessibility settings open for you to turn my-domain on.
 *
 * Secure mode stays on until the helper really is on again (so it shows on
 * while you're on the way to Settings). The switch is in Settings → Control
 * from your computers, on the home screen while the helper is on, and in
 * Quick Settings ([SecureModeTileService]).
 */
object SecureMode {
    private const val TAG = "SecureMode"
    private const val PREF_WANTED = "secure_mode"
    /** When Secure mode was asked for: a helper that connects just after
     *  (it was still starting) switches off instead of cancelling it. */
    private const val PREF_ASKED_AT = "secure_mode_at"
    private const val PREF_IDLE = "secure_auto_idle_min"
    private const val PREF_AFTER = "secure_auto_after_control"
    private const val PREF_CARD_HIDDEN = "secure_card_hidden"
    private const val STARTING_MS = 10_000L

    const val WRITE_SECURE = "android.permission.WRITE_SECURE_SETTINGS"
    /** What you run once on your computer for instant switching. */
    const val ADB_GRANT = "adb shell pm grant com.mydomain.android android.permission.WRITE_SECURE_SETTINGS"

    private const val CHANNEL = "phone_helper"
    private const val NOTE_NEEDED = 7801
    private var lastTold = 0L
    private val main = Handler(Looper.getMainLooper())

    private fun prefs(ctx: Context) = ctx.getSharedPreferences("md", Context.MODE_PRIVATE)

    fun component(ctx: Context) = ComponentName(ctx, PhoneHelper::class.java)

    /** You asked for Secure mode (and the helper isn't back on since). */
    fun wanted(ctx: Context): Boolean = prefs(ctx).getBoolean(PREF_WANTED, false)

    fun state(ctx: Context): SecureModePlan.State = SecureModePlan.state(wanted(ctx), PhoneHelper.enabled(ctx))

    /** The switch shows Secure mode on (the helper is off, or going off). */
    fun isOn(ctx: Context): Boolean = SecureModePlan.shownOn(state(ctx))

    /** You granted WRITE_SECURE_SETTINGS over adb: switching back is instant. */
    fun canWriteSecure(ctx: Context): Boolean =
        ctx.checkSelfPermission(WRITE_SECURE) == PackageManager.PERMISSION_GRANTED

    fun idleMinutes(ctx: Context): Int = SecureModePlan.idleMinutes(prefs(ctx).getInt(PREF_IDLE, 0))

    fun setIdleMinutes(ctx: Context, min: Int) {
        prefs(ctx).edit().putInt(PREF_IDLE, SecureModePlan.idleMinutes(min)).apply()
        PhoneHelper.instance?.scheduleIdle()
    }

    fun afterSession(ctx: Context): Boolean = prefs(ctx).getBoolean(PREF_AFTER, false)

    fun setAfterSession(ctx: Context, on: Boolean) = prefs(ctx).edit().putBoolean(PREF_AFTER, on).apply()

    fun cardHidden(ctx: Context): Boolean = prefs(ctx).getBoolean(PREF_CARD_HIDDEN, false)

    fun hideCard(ctx: Context) = prefs(ctx).edit().putBoolean(PREF_CARD_HIDDEN, true).apply()

    /** Turn Secure mode on: the helper switches itself off. */
    fun turnOn(ctx: Context) {
        val app = ctx.applicationContext
        prefs(app).edit().putBoolean(PREF_WANTED, true).putLong(PREF_ASKED_AT, System.currentTimeMillis()).apply()
        val helper = PhoneHelper.instance
        when {
            helper != null -> helper.switchOff()
            // On in Android's list but not running yet.
            PhoneHelper.enabled(app) && canWriteSecure(app) -> writeList(app, on = false)
        }
        Log.i(TAG, "on")
        changed(app)
    }

    /** Turn Secure mode off: the helper back on, at once if this app may
     *  write Android's secure settings; else [open] gets Android's
     *  Accessibility settings (with a hint to turn my-domain on). */
    fun turnOff(ctx: Context, open: (Intent) -> Unit = { ctx.startActivity(it) }): SecureModePlan.Path {
        val app = ctx.applicationContext
        // The helper that connects next is you turning it on.
        prefs(app).edit().putLong(PREF_ASKED_AT, 0).apply()
        val path = SecureModePlan.enablePath(PhoneHelper.enabled(app), canWriteSecure(app))
        when (path) {
            SecureModePlan.Path.NONE -> prefs(app).edit().putBoolean(PREF_WANTED, false).apply()
            SecureModePlan.Path.WRITE -> {
                if (writeList(app, on = true)) {
                    prefs(app).edit().putBoolean(PREF_WANTED, false).apply()
                } else {
                    openSettings(app, open)
                    return SecureModePlan.Path.SETTINGS
                }
            }
            SecureModePlan.Path.SETTINGS -> openSettings(app, open)
        }
        Log.i(TAG, "off: $path")
        changed(app)
        return path
    }

    private fun openSettings(app: Context, open: (Intent) -> Unit) {
        if (runCatching { open(settingsIntent(app)) }.isFailure) {
            runCatching { open(Intent(Settings.ACTION_ACCESSIBILITY_SETTINGS).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)) }
        }
        main.post { Toast.makeText(app, app.getString(R.string.secure_turn_on_hint), Toast.LENGTH_LONG).show() }
    }

    /** Android's Accessibility settings; on Android 11+ scrolled to (and
     *  on some phones opened on) my-domain's entry. */
    fun settingsIntent(ctx: Context): Intent {
        val i = Intent(Settings.ACTION_ACCESSIBILITY_SETTINGS).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
            val key = component(ctx).flattenToString()
            i.putExtra(FRAGMENT_ARGS_KEY, key)
            i.putExtra(SHOW_FRAGMENT_ARGS, Bundle().apply { putString(FRAGMENT_ARGS_KEY, key) })
        }
        return i
    }

    private const val FRAGMENT_ARGS_KEY = ":settings:fragment_args_key"
    private const val SHOW_FRAGMENT_ARGS = ":settings:show_fragment_args"

    /** Put the helper in (or take it out of) Android's list of enabled
     *  Accessibility services, every other service kept. Needs
     *  WRITE_SECURE_SETTINGS, which only you can grant (adb). */
    private fun writeList(app: Context, on: Boolean): Boolean = runCatching {
        val cr = app.contentResolver
        val me = component(app).flattenToString()
        val now = Settings.Secure.getString(cr, Settings.Secure.ENABLED_ACCESSIBILITY_SERVICES)
        val next = if (on) SecureModePlan.add(now, me) else SecureModePlan.remove(now, me)
        Settings.Secure.putString(cr, Settings.Secure.ENABLED_ACCESSIBILITY_SERVICES, next)
        if (on) Settings.Secure.putInt(cr, Settings.Secure.ACCESSIBILITY_ENABLED, 1)
        true
    }.onFailure { Log.w(TAG, "write: $it") }.getOrDefault(false)

    /** The helper connected: you turned it on (Secure mode ends), unless
     *  Secure mode was asked for while it was still starting (it should
     *  switch off: true). */
    fun onHelperConnected(ctx: Context): Boolean {
        val p = prefs(ctx)
        if (!p.getBoolean(PREF_WANTED, false)) return false.also { changed(ctx) }
        if (System.currentTimeMillis() - p.getLong(PREF_ASKED_AT, 0) < STARTING_MS) return true
        p.edit().putBoolean(PREF_WANTED, false).apply()
        changed(ctx)
        return false
    }

    /** Something changed: the status, the tile and the notification follow. */
    fun changed(ctx: Context) {
        val app = ctx.applicationContext
        Controls.onChange()
        runCatching { TileService.requestListeningState(app, ComponentName(app, SecureModeTileService::class.java)) }
        if (!wanted(app)) runCatching { app.getSystemService(NotificationManager::class.java)?.cancel(NOTE_NEEDED) }
    }

    /** A computer ([fromName]) asked for something that needs the helper
     *  (taps, a key, a radio switch) and it's off: if that's Secure mode,
     *  say so here with a "Turn helper on" button. True if Secure mode is
     *  why (the computer reads `secure_mode` in the status). */
    fun blocked(ctx: Context, fromName: String): Boolean {
        val app = ctx.applicationContext
        if (PhoneHelper.instance != null || !isOn(app)) return false
        val now = System.currentTimeMillis()
        if (!SecureModePlan.shouldTell(lastTold, now)) return true
        lastTold = now
        channel(app)
        val turnOn = PendingIntent.getActivity(
            app, NOTE_NEEDED,
            Intent(app, SecureModeActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
        val n = NotificationCompat.Builder(app, CHANNEL)
            .setSmallIcon(android.R.drawable.ic_lock_idle_lock)
            .setContentTitle(app.getString(R.string.secure_needed_title))
            .setContentText(app.getString(R.string.secure_needed_body, fromName.ifEmpty { app.getString(R.string.secure_a_computer) }))
            .setStyle(
                NotificationCompat.BigTextStyle().bigText(
                    app.getString(R.string.secure_needed_body, fromName.ifEmpty { app.getString(R.string.secure_a_computer) }),
                ),
            )
            .setAutoCancel(true)
            .addAction(0, app.getString(R.string.secure_turn_helper_on), turnOn)
            .build()
        runCatching { app.getSystemService(NotificationManager::class.java)?.notify(NOTE_NEEDED, n) }
            .onFailure { Log.w(TAG, "can't tell: $it") }
        return true
    }

    private fun channel(ctx: Context) {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.O) return
        runCatching {
            ctx.getSystemService(NotificationManager::class.java)?.createNotificationChannel(
                NotificationChannel(CHANNEL, ctx.getString(R.string.channel_phone_helper), NotificationManager.IMPORTANCE_DEFAULT),
            )
        }
    }

    /** Taps and swipes from a computer were allowed and now aren't (you
     *  switched it off, or the share ended): Secure mode on, if you asked
     *  for that. */
    fun controlEnded(ctx: Context, wasAllowed: Boolean) {
        val app = ctx.applicationContext
        PhoneHelper.instance?.markUse()
        if (SecureModePlan.afterSessionDue(afterSession(app), wasAllowed) && PhoneHelper.instance != null) {
            Log.i(TAG, "control ended: on")
            turnOn(app)
        }
    }
}

/** Quick Settings tile: Secure mode on (the helper off, so banking apps
 *  work) or off (the helper on). */
class SecureModeTileService : TileService() {
    override fun onStartListening() = refresh()

    override fun onClick() {
        if (isLocked) unlockAndRun { toggle() } else toggle()
    }

    private fun toggle() {
        if (!SecureMode.isOn(this)) {
            SecureMode.turnOn(this)
        } else {
            SecureMode.turnOff(this) { open -> collapseTo(open) }
        }
        refresh()
    }

    /** Close the panel and show [open]. */
    private fun collapseTo(open: Intent) {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.UPSIDE_DOWN_CAKE) {
            val pi = PendingIntent.getActivity(this, 0, open, PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
            startActivityAndCollapse(pi)
        } else {
            @Suppress("DEPRECATION")
            startActivityAndCollapse(open)
        }
    }

    private fun refresh() {
        val tile = qsTile ?: return
        val on = SecureMode.isOn(this)
        tile.state = if (on) Tile.STATE_ACTIVE else Tile.STATE_INACTIVE
        tile.label = getString(R.string.secure_title)
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
            tile.subtitle = getString(if (PhoneHelper.enabled(this) && !on) R.string.secure_tile_helper_on else R.string.secure_tile_helper_off)
        }
        tile.updateTile()
    }
}

/** "Turn helper on" in the notification: Secure mode off (opens Android's
 *  Accessibility settings when it can't be instant), then closes. */
class SecureModeActivity : Activity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        val path = SecureMode.turnOff(this)
        if (path != SecureModePlan.Path.SETTINGS) {
            Toast.makeText(applicationContext, getString(R.string.secure_helper_back_on), Toast.LENGTH_SHORT).show()
        }
        finish()
        overridePendingTransition(0, 0)
    }
}
