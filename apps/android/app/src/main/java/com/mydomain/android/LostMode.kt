package com.mydomain.android

import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.heading
import android.app.KeyguardManager
import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.content.SharedPreferences
import android.net.Uri
import android.os.Build
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.view.WindowManager
import androidx.activity.ComponentActivity
import androidx.activity.addCallback
import androidx.activity.compose.setContent
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Button
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.darkColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.core.app.NotificationCompat
import com.mydomain.android.core.Core
import org.json.JSONObject
import java.lang.ref.WeakReference

/**
 * Lost mode, started from one of your computers (`lost {on, message?,
 * contact?}`): your message full screen over the lock screen, a "Call owner"
 * button that opens the dialer with your number (the finder presses Call),
 * one loud ring like `ring`, and one location share if — and only if — you
 * turned on location sharing on the phone (see PhoneLocation).
 *
 * It never unlocks the phone: the screen shows over the lock screen, and
 * everything behind it stays locked. It stays on until a computer sends
 * `lost {on: false}`, or the owner taps "I found it", which asks Android to
 * unlock (the PIN) before it turns lost mode off.
 *
 * The state is kept in device-protected storage, so after a restart the
 * message is back before the phone has been unlocked; it holds only what the
 * finder sees anyway. The status says `lost: true` while it's on.
 */
object LostMode {
    const val CH_LOST = "lost"
    private const val NOTE_ID = 7701
    private const val MAX_MESSAGE = 500
    private const val MAX_CONTACT = 20
    private const val KEY_ON = "on"
    private const val KEY_MESSAGE = "message"
    private const val KEY_CONTACT = "contact"

    private val main = Handler(Looper.getMainLooper())
    /** The lost screen while it exists, so turning lost mode off closes it. */
    @Volatile internal var screen: WeakReference<LostModeActivity>? = null
    private var screenReceiver: BroadcastReceiver? = null

    private fun prefs(ctx: Context): SharedPreferences =
        ctx.applicationContext.createDeviceProtectedStorageContext()
            .getSharedPreferences("lost_mode", Context.MODE_PRIVATE)

    fun isOn(ctx: Context): Boolean = prefs(ctx).getBoolean(KEY_ON, false)
    fun message(ctx: Context): String = prefs(ctx).getString(KEY_MESSAGE, null).orEmpty()
    fun contact(ctx: Context): String = prefs(ctx).getString(KEY_CONTACT, null).orEmpty()

    /** `lost: true` in the status while it's on. */
    fun state(ctx: Context, o: JSONObject) {
        if (isOn(ctx)) o.put("lost", true)
    }

    /** Another device's `lost` action. `on` is required, so an empty
     *  `lost {}` does nothing. */
    fun run(ctx: Context, args: JSONObject, fromName: String): Boolean {
        val app = ctx.applicationContext
        if (!args.has("on")) return false
        if (!args.optBoolean("on")) {
            stop(app)
            return true
        }
        val message = (if (args.isNull("message")) "" else args.optString("message")).trim().take(MAX_MESSAGE)
        val contact = (if (args.isNull("contact")) "" else args.optString("contact")).filter { it.isDigit() || it in "+*#" }
        if (contact.length > MAX_CONTACT) return false
        prefs(app).edit()
            .putBoolean(KEY_ON, true)
            .putString(KEY_MESSAGE, message)
            .putString(KEY_CONTACT, contact)
            .commit()
        show(app, Core.visible || PhoneHelper.instance != null)
        DeviceActions.ring(app, fromName)
        PhoneLocation.locateQuietly(app, fromName)
        return true
    }

    /** Lost mode off: forget the message, close the screen, stop the ring. */
    fun stop(ctx: Context) {
        val app = ctx.applicationContext
        prefs(app).edit().clear().commit()
        runCatching { app.getSystemService(NotificationManager::class.java)?.cancel(NOTE_ID) }
        DeviceActions.stopRing()
        main.post { screen?.get()?.finish() }
    }

    /**
     * Put the lost screen up (when it's on and not already showing): at once
     * when Android lets this app open a screen ([direct]: on screen, or the
     * Accessibility helper on), and always an ongoing notification whose
     * full-screen intent opens it over the lock screen. The notification
     * stays, as a way back to the screen.
     */
    fun show(ctx: Context, direct: Boolean) {
        val app = ctx.applicationContext
        if (!isOn(app)) return
        val open = screen?.get()
        if (open != null && open.isOnScreen) {
            main.post { open.load() } // a new message, if it changed
            return
        }
        if (direct) runCatching { app.startActivity(screenIntent(app)) }
        notifyLost(app)
    }

    /** From the app (it's on screen, so it may open it straight away). */
    fun open(ctx: Context) {
        runCatching { ctx.startActivity(screenIntent(ctx)) }
    }

    private fun screenIntent(ctx: Context): Intent =
        Intent(ctx, LostModeActivity::class.java)
            .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_SINGLE_TOP)

    private fun notifyLost(app: Context) {
        val nm = app.getSystemService(NotificationManager::class.java) ?: return
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            // No channel sound: the ring is its own player (DeviceActions).
            val ch = NotificationChannel(CH_LOST, app.getString(R.string.channel_lost), NotificationManager.IMPORTANCE_HIGH).apply {
                setSound(null, null)
                lockscreenVisibility = Notification.VISIBILITY_PUBLIC
            }
            runCatching { nm.createNotificationChannel(ch) }
        }
        val pi = PendingIntent.getActivity(
            app, NOTE_ID, screenIntent(app), PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
        val n = NotificationCompat.Builder(app, CH_LOST)
            .setSmallIcon(android.R.drawable.ic_lock_lock)
            .setContentTitle(app.getString(R.string.lost_title))
            .setContentText(message(app).ifEmpty { app.getString(R.string.lost_notification_body) })
            .setPriority(NotificationCompat.PRIORITY_MAX)
            .setCategory(NotificationCompat.CATEGORY_ALARM)
            .setVisibility(NotificationCompat.VISIBILITY_PUBLIC)
            .setOngoing(true)
            .setContentIntent(pi)
            .setFullScreenIntent(pi, true)
            .build()
        // Posted anew (not updated), so the full-screen intent fires again.
        runCatching {
            nm.cancel(NOTE_ID)
            nm.notify(NOTE_ID, n)
        }
    }

    /** NetService: bring the screen back each time the screen turns on. */
    fun watchScreen(ctx: Context) {
        if (screenReceiver != null) return
        val r = object : BroadcastReceiver() {
            override fun onReceive(c: Context, i: Intent) {
                show(c, PhoneHelper.instance != null)
            }
        }
        runCatching { ctx.applicationContext.registerReceiver(r, IntentFilter(Intent.ACTION_SCREEN_ON)) }
            .onSuccess { screenReceiver = r }
    }

    fun unwatchScreen(ctx: Context) {
        screenReceiver?.let { runCatching { ctx.applicationContext.unregisterReceiver(it) } }
        screenReceiver = null
    }
}

/**
 * The lost screen: shown over the lock screen (and turning the screen on),
 * never past it. Back does nothing (it would only uncover the lock screen);
 * "I found it" asks Android to unlock first. Not MdTheme: that reads the
 * app's settings, which stay locked away until the first unlock after a
 * restart, and this screen shows before then too.
 */
class LostModeActivity : ComponentActivity() {
    private var message by mutableStateOf("")
    private var contact by mutableStateOf("")
    internal var isOnScreen = false
        private set

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O_MR1) {
            setShowWhenLocked(true)
            setTurnScreenOn(true)
        } else {
            @Suppress("DEPRECATION")
            window.addFlags(WindowManager.LayoutParams.FLAG_SHOW_WHEN_LOCKED or WindowManager.LayoutParams.FLAG_TURN_SCREEN_ON)
        }
        LostMode.screen = WeakReference(this)
        onBackPressedDispatcher.addCallback(this) { }
        load()
        setContent { LostScreen(message, contact, onCall = ::call, onFound = ::found) }
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        load()
    }

    override fun onStart() {
        super.onStart()
        isOnScreen = true
    }

    override fun onResume() {
        super.onResume()
        if (!LostMode.isOn(this)) finish() else load()
    }

    override fun onStop() {
        super.onStop()
        isOnScreen = false
    }

    override fun onDestroy() {
        super.onDestroy()
        if (LostMode.screen?.get() === this) LostMode.screen = null
    }

    internal fun load() {
        message = LostMode.message(this)
        contact = LostMode.contact(this)
    }

    /** The dialer with the owner's number in; the finder presses Call (no
     *  call permission). */
    private fun call() {
        val number = contact.takeIf { it.isNotEmpty() } ?: return
        runCatching {
            startActivity(Intent(Intent.ACTION_DIAL, Uri.parse("tel:" + Uri.encode(number))).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
        }
    }

    /** "I found it": locked, Android asks for the PIN first, and only then
     *  is lost mode turned off. */
    private fun found() {
        val km = getSystemService(KeyguardManager::class.java)
        if (km != null && km.isKeyguardLocked) {
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
                km.requestDismissKeyguard(this, object : KeyguardManager.KeyguardDismissCallback() {
                    override fun onDismissSucceeded() = turnOff()
                })
            } else {
                // Android 7 can't ask for the PIN from here: show the lock
                // screen; once unlocked, opening my-domain brings this back.
                finish()
            }
            return
        }
        turnOff()
    }

    private fun turnOff() {
        Core.init(this)
        LostMode.stop(this)
        Controls.onChange()
        finish()
    }
}

@Composable
private fun LostScreen(message: String, contact: String, onCall: () -> Unit, onFound: () -> Unit) {
    MaterialTheme(colorScheme = darkColorScheme()) {
        Box(Modifier.fillMaxSize().background(Color(0xFF05060F)), contentAlignment = Alignment.Center) {
            Column(
                Modifier.verticalScroll(rememberScrollState()).padding(28.dp),
                verticalArrangement = Arrangement.Center,
                horizontalAlignment = Alignment.CenterHorizontally,
            ) {
                Text(
                    stringResource(R.string.lost_title), Modifier.semantics { heading() }, style = MaterialTheme.typography.headlineMedium,
                    color = Color.White, fontWeight = FontWeight.Bold, textAlign = TextAlign.Center,
                )
                Spacer(Modifier.height(16.dp))
                val fallback = stringResource(R.string.lost_default_message)
                Text(
                    message.ifEmpty { fallback },
                    style = MaterialTheme.typography.titleLarge, color = Color.White, textAlign = TextAlign.Center,
                )
                if (contact.isNotEmpty()) {
                    Spacer(Modifier.height(28.dp))
                    Text(
                        contact, style = MaterialTheme.typography.headlineLarge,
                        color = Color.White, fontWeight = FontWeight.Bold, textAlign = TextAlign.Center,
                    )
                    Spacer(Modifier.height(16.dp))
                    Button(onClick = onCall, modifier = Modifier.fillMaxWidth().height(56.dp)) {
                        Text(stringResource(R.string.lost_call), style = MaterialTheme.typography.titleMedium)
                    }
                }
                Spacer(Modifier.height(48.dp))
                TextButton(onClick = onFound) { Text(stringResource(R.string.lost_found), color = Color(0xFFB0B4C8)) }
                Text(
                    stringResource(R.string.lost_found_hint), style = MaterialTheme.typography.bodySmall,
                    color = Color(0xFF8A8FA8), textAlign = TextAlign.Center,
                )
            }
        }
    }
}

/**
 * After a restart: the lost screen back on the lock screen before the first
 * unlock (LOCKED_BOOT_COMPLETED, device-protected storage), and once
 * unlocked the service, so your computers can reach the phone and stop it.
 */
class LostModeBootReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        if (!LostMode.isOn(context)) return
        when (intent.action) {
            Intent.ACTION_LOCKED_BOOT_COMPLETED -> LostMode.show(context, false)
            Intent.ACTION_BOOT_COMPLETED -> {
                // The service shows it again when it starts (NetService).
                val svc = Intent(context, NetService::class.java)
                val started = runCatching {
                    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) context.startForegroundService(svc) else context.startService(svc)
                }.isSuccess
                if (!started) LostMode.show(context, false)
            }
        }
    }
}
