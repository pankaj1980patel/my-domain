package com.mydomain.android

import android.content.Intent
import android.os.Build
import android.os.PowerManager
import android.os.SystemClock
import android.util.Log
import com.google.firebase.messaging.FirebaseMessagingService
import com.google.firebase.messaging.RemoteMessage
import com.mydomain.android.core.Core
import com.mydomain.android.core.EngineState
import com.mydomain.android.core.ffi
import com.mydomain.android.core.ResourceBook
import com.mydomain.android.core.ResourceLedger

/**
 * Receives connection signals pushed through FCM and feeds them to the core
 * (`Engine::on_signal`), the same path the desktop's FCM receiver uses.
 * If the app was asleep, the push starts NetService (and with it the engine)
 * first: a high-priority FCM message may start a foreground service.
 */
class MdMessagingService : FirebaseMessagingService() {
    override fun onNewToken(token: String) = Fcm.publish(token)

    override fun onMessageReceived(message: RemoteMessage) {
        val got = SystemClock.elapsedRealtime()
        val data = message.data
        // `from` is reserved by FCM, so the server sends the sender as `sender`.
        val sender = data["sender"] ?: data["from"] ?: return
        val payload = data["payload"] ?: return
        // A normal-priority push (or a high one Android demoted for this
        // app) can't start a service from the background on Android 12+
        // for apps targeting it; this app targets 28, so it can, but say
        // which it was: it explains a slow wake.
        val prio = if (message.priority == RemoteMessage.PRIORITY_HIGH) "high" else "normal"
        val sent = if (message.originalPriority == message.priority) prio else "$prio (sent high)"
        Log.i(TAG, "signal from ${sender.take(8)}: ${data["type"]}, $sent")
        Core.labLog("push from ${sender.take(8)}: ${data["type"]}, $sent")
        stayAwake()
        if (!waitForEngine()) {
            Log.w(TAG, "signal from $sender dropped: engine not ready (${Core.engineState.value})")
            Core.labLog("push dropped: engine ${Core.engineState.value}")
            return
        }
        Core.engine()?.let { engine ->
            // The link this brings is timed from when the push came, the
            // engine's start included (the connection report).
            ffi("pushReceived") { engine.pushReceived(sender, (SystemClock.elapsedRealtime() - got).toULong()) }
            ffi("onSignal") { engine.onSignal(sender, payload) }
        }
    }

    /** Start the engine if needed; true once it's logged in and ready. */
    private fun waitForEngine(): Boolean {
        if (ready()) return true
        // A high-priority push may start a foreground service from the
        // background (and this app targets 28, so Android 12's limits on
        // that don't apply to it either).
        // Android 7 has no startForegroundService (the call threw, unseen,
        // and every push was dropped): a plain start there, which Android 7
        // allows from the background. A service whose engine failed to
        // start tries it again at once.
        val svc = Intent(this, NetService::class.java)
        runCatching {
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) startForegroundService(svc) else startService(svc)
        }.onFailure { Log.w(TAG, "can't start NetService: $it") }
        // onMessageReceived runs on a worker thread and may block briefly;
        // look often at first (a warm start takes a few hundred ms).
        repeat(100) {
            if (ready()) return true
            Thread.sleep(100)
        }
        return false
    }

    /**
     * FCM keeps the phone awake only while this handler runs; the link the
     * signal sets up, and the message waiting on it, need a little longer.
     * Without this the CPU sleeps mid-connect (screen off) and the sender
     * sees a link that never answers. Times out by itself.
     */
    private fun stayAwake() {
        val pm = getSystemService(PowerManager::class.java) ?: return
        runCatching {
            val lock = synchronized(MdMessagingService::class.java) {
                pushLock ?: pm.newWakeLock(PowerManager.PARTIAL_WAKE_LOCK, "mydomain:push")
                    .apply { setReferenceCounted(false) }
                    .also { pushLock = it }
            }
            // Not reference counted: another push restarts the timeout.
            lock.acquire(AWAKE_AFTER_PUSH_MS)
            ResourceLedger.hold(ResourceBook.Kind.WAKE_LOCK, "mydomain:push", AWAKE_AFTER_PUSH_MS)
        }
    }

    /** The engine is up ([Core.engineState]) and signed in. */
    private fun ready() = Core.engineState.value == EngineState.Ready &&
        ffi("isReady") { Core.engine()?.isReady() } == true

    private companion object {
        /** The one wake lock pushes take (times out by itself). */
        var pushLock: PowerManager.WakeLock? = null
        const val TAG = "mydomain_fcm"
        const val AWAKE_AFTER_PUSH_MS = 30_000L
    }
}
