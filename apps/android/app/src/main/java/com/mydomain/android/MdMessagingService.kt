package com.mydomain.android

import android.content.Intent
import android.os.PowerManager
import android.util.Log
import com.google.firebase.messaging.FirebaseMessagingService
import com.google.firebase.messaging.RemoteMessage
import com.mydomain.android.core.Core

/**
 * Receives connection signals pushed through FCM and feeds them to the core
 * (`Engine::on_signal`), the same path the desktop's FCM receiver uses.
 * If the app was asleep, the push starts NetService (and with it the engine)
 * first: a high-priority FCM message may start a foreground service.
 */
class MdMessagingService : FirebaseMessagingService() {
    override fun onNewToken(token: String) = Fcm.publish(token)

    override fun onMessageReceived(message: RemoteMessage) {
        val data = message.data
        // `from` is reserved by FCM, so the server sends the sender as `sender`.
        val sender = data["sender"] ?: data["from"] ?: return
        val payload = data["payload"] ?: return
        Log.i(TAG, "signal from ${sender.take(8)}: ${data["type"]}")
        Core.labLog("push from ${sender.take(8)}: ${data["type"]}")
        stayAwake()
        if (!waitForEngine()) {
            Log.w(TAG, "signal from $sender dropped: engine not ready")
            return
        }
        Core.engine()?.onSignal(sender, payload)
    }

    /** Start the engine if needed; true once it's logged in and ready. */
    private fun waitForEngine(): Boolean {
        if (ready()) return true
        runCatching { startForegroundService(Intent(this, NetService::class.java)) }
            .onFailure { Log.w(TAG, "can't start NetService: $it") }
        // onMessageReceived runs on a worker thread and may block briefly.
        repeat(50) {
            if (ready()) return true
            Thread.sleep(200)
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
            pm.newWakeLock(PowerManager.PARTIAL_WAKE_LOCK, "mydomain:push").acquire(AWAKE_AFTER_PUSH_MS)
        }
    }

    private fun ready() = runCatching { Core.engine()?.isReady() == true }.getOrDefault(false)

    private companion object {
        const val TAG = "mydomain_fcm"
        const val AWAKE_AFTER_PUSH_MS = 30_000L
    }
}
