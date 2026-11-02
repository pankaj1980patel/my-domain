package com.mydomain.android

import android.content.Context
import android.util.Log
import com.google.firebase.FirebaseApp
import com.google.firebase.FirebaseOptions
import com.google.firebase.messaging.FirebaseMessaging

/**
 * FCM setup. Firebase is initialised in code from ids baked into BuildConfig
 * at build time (see `firebaseIds` in build.gradle.kts), not by the
 * google-services plugin, so a missing config just leaves FCM off.
 *
 * FCM is how other devices reach this phone when it has no live link: the
 * server forwards their signal as a push, which costs nothing to run.
 */
object Fcm {
    private const val TAG = "mydomain_fcm"

    fun init(context: Context) {
        if (BuildConfig.FCM_APP_ID.isEmpty()) {
            Log.w(TAG, "no Firebase config at build time; FCM off")
            return
        }
        runCatching {
            if (FirebaseApp.getApps(context).isEmpty()) {
                val options = FirebaseOptions.Builder()
                    .setApplicationId(BuildConfig.FCM_APP_ID)
                    .setApiKey(BuildConfig.FCM_API_KEY)
                    .setProjectId(BuildConfig.FCM_PROJECT_ID)
                    .setGcmSenderId(BuildConfig.FCM_SENDER_ID)
                    .build()
                FirebaseApp.initializeApp(context, options)
            }
            FirebaseMessaging.getInstance().token
                .addOnSuccessListener { publish(it) }
                .addOnFailureListener { Log.w(TAG, "token failed: $it") }
        }.onFailure { Log.w(TAG, "init failed: $it") }
    }

    /** Hand the token to the core, which registers it with the server. */
    fun publish(token: String) {
        Log.i(TAG, "token ready (${token.take(8)}…)")
        // nativeSetFcmToken re-registers over the network: keep it off the
        // main thread.
        Thread { RustNet.nativeSetFcmToken(token) }.start()
    }
}
