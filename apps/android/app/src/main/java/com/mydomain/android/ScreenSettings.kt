package com.mydomain.android

import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.database.ContentObserver
import android.net.Uri
import android.os.Handler
import android.os.Looper
import android.provider.Settings
import android.util.Log
import androidx.core.app.NotificationCompat
import com.mydomain.android.core.Core
import org.json.JSONObject

/**
 * The phone's screen settings for your computers: brightness (0–100, or
 * automatic), how long the screen stays on, and auto-rotate.
 *
 * In the status: `brightness`, `auto_brightness`, `screen_timeout_s`,
 * `auto_rotate`, and `write_settings` (whether this app may change them).
 * The `screen_settings {brightness?, auto_brightness?, timeout_s?, rotate?}`
 * action changes the ones it names.
 *
 * Reading these needs nothing; changing them needs "Modify system settings"
 * (WRITE_SETTINGS), which only the user can grant, on a system screen.
 * Asked to change something without it, the phone opens that screen (or
 * posts a notification that does) and changes nothing.
 */
object ScreenSettings {
    private const val TAG = "mydomain_screen_set"
    private const val NOTE_ASK = 7701
    /** Screen timeouts accepted, in seconds (5 s to 30 min). */
    private const val MIN_TIMEOUT_S = 5
    private const val MAX_TIMEOUT_S = 30 * 60
    /** After asking, watch this long for the permission to be granted. */
    private const val WATCH_GRANT_MS = 3 * 60_000L

    private val KEYS = listOf(
        Settings.System.SCREEN_BRIGHTNESS,
        Settings.System.SCREEN_BRIGHTNESS_MODE,
        Settings.System.SCREEN_OFF_TIMEOUT,
        Settings.System.ACCELEROMETER_ROTATION,
    )

    private val main = Handler(Looper.getMainLooper())
    private var observer: ContentObserver? = null
    /** One status push for a burst of changes (dragging the slider). */
    private val push = Runnable { Controls.onChange() }

    /** Tell [Controls.onChange] when any of these settings changes, by anyone. */
    fun init(ctx: Context) {
        if (observer != null) return
        val obs = object : ContentObserver(main) {
            override fun onChange(selfChange: Boolean) {
                main.removeCallbacks(push)
                main.postDelayed(push, 300)
            }
        }
        val cr = ctx.applicationContext.contentResolver
        runCatching {
            for (k in KEYS) cr.registerContentObserver(Settings.System.getUriFor(k), false, obs)
        }
            .onSuccess { observer = obs }
            .onFailure { Log.w(TAG, "can't watch the screen settings: $it") }
    }

    fun canWrite(ctx: Context): Boolean = runCatching { Settings.System.canWrite(ctx) }.getOrDefault(false)

    /** The current settings, into the status. */
    fun state(ctx: Context, o: JSONObject) {
        val cr = ctx.contentResolver
        runCatching { Settings.System.getInt(cr, Settings.System.SCREEN_BRIGHTNESS) }.getOrNull()
            ?.let { o.put("brightness", (it.coerceIn(0, 255) * 100 + 127) / 255) }
        runCatching { Settings.System.getInt(cr, Settings.System.SCREEN_BRIGHTNESS_MODE) }.getOrNull()
            ?.let { o.put("auto_brightness", it == Settings.System.SCREEN_BRIGHTNESS_MODE_AUTOMATIC) }
        runCatching { Settings.System.getInt(cr, Settings.System.SCREEN_OFF_TIMEOUT) }.getOrNull()
            ?.let { o.put("screen_timeout_s", it / 1000) }
        runCatching { Settings.System.getInt(cr, Settings.System.ACCELEROMETER_ROTATION) }.getOrNull()
            ?.let { o.put("auto_rotate", it == 1) }
        o.put("write_settings", canWrite(ctx))
    }

    /** Change the settings [args] names; false if nothing could be changed. */
    fun run(ctx: Context, args: JSONObject): Boolean {
        val app = ctx.applicationContext
        if (!canWrite(app)) {
            ask(app)
            return false
        }
        val cr = app.contentResolver
        var ok = true
        fun put(key: String, value: Int) {
            val done = runCatching { Settings.System.putInt(cr, key, value) }
                .onFailure { Log.w(TAG, "can't set $key: $it") }
                .getOrDefault(false)
            if (!done) ok = false
        }
        if (args.has("auto_brightness")) {
            put(
                Settings.System.SCREEN_BRIGHTNESS_MODE,
                if (args.optBoolean("auto_brightness")) Settings.System.SCREEN_BRIGHTNESS_MODE_AUTOMATIC
                else Settings.System.SCREEN_BRIGHTNESS_MODE_MANUAL,
            )
        }
        if (args.has("brightness")) {
            // With automatic brightness on, the level does nothing visible,
            // so choosing a level turns automatic off (unless asked too).
            if (!args.has("auto_brightness")) {
                put(Settings.System.SCREEN_BRIGHTNESS_MODE, Settings.System.SCREEN_BRIGHTNESS_MODE_MANUAL)
            }
            val level = args.optInt("brightness", 50).coerceIn(0, 100)
            // Not 0: on some phones that's a black screen.
            put(Settings.System.SCREEN_BRIGHTNESS, (level * 255 / 100).coerceIn(1, 255))
        }
        if (args.has("timeout_s")) {
            val s = args.optInt("timeout_s", 30).coerceIn(MIN_TIMEOUT_S, MAX_TIMEOUT_S)
            put(Settings.System.SCREEN_OFF_TIMEOUT, s * 1000)
        }
        if (args.has("rotate")) {
            put(Settings.System.ACCELEROMETER_ROTATION, if (args.optBoolean("rotate")) 1 else 0)
        }
        return ok
    }

    /** Open "Modify system settings" for this app straight away when Android
     *  lets it open a screen, else a notification that opens it. Then watch
     *  for the grant, so your computers see it without asking again. */
    private fun ask(app: Context) {
        val open = Intent(Settings.ACTION_MANAGE_WRITE_SETTINGS, Uri.parse("package:" + app.packageName))
            .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
        watchGrant(app, System.currentTimeMillis() + WATCH_GRANT_MS)
        if ((Core.visible || PhoneHelper.instance != null) && runCatching { app.startActivity(open) }.isSuccess) return
        val text = app.getString(R.string.screen_settings_ask_body)
        val pi = PendingIntent.getActivity(app, NOTE_ASK, open, PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
        val n = NotificationCompat.Builder(app, DeviceActions.CH_LINKS)
            .setSmallIcon(android.R.drawable.ic_menu_manage)
            .setContentTitle(app.getString(R.string.screen_settings_ask_title))
            .setContentText(text)
            .setStyle(NotificationCompat.BigTextStyle().bigText(text))
            .setAutoCancel(true)
            .setContentIntent(pi)
            .build()
        runCatching { app.getSystemService(NotificationManager::class.java)?.notify(NOTE_ASK, n) }
    }

    @Volatile
    private var watchingUntil = 0L

    /** Android says nothing when the permission is granted: check every few
     *  seconds until [until], and push a status once it is. */
    private fun watchGrant(app: Context, until: Long) {
        val already = watchingUntil > System.currentTimeMillis()
        watchingUntil = until
        if (already) return
        val check = object : Runnable {
            override fun run() {
                if (canWrite(app)) {
                    watchingUntil = 0L
                    runCatching { app.getSystemService(NotificationManager::class.java)?.cancel(NOTE_ASK) }
                    Controls.onChange()
                } else if (System.currentTimeMillis() < watchingUntil) {
                    main.postDelayed(this, 2_000)
                } else {
                    watchingUntil = 0L
                }
            }
        }
        main.postDelayed(check, 2_000)
    }
}
