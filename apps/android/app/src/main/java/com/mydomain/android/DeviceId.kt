package com.mydomain.android

import android.Manifest
import android.content.Context
import android.content.pm.PackageManager
import android.os.Build
import android.provider.Settings
import android.telephony.TelephonyManager
import android.util.Log
import androidx.core.content.ContextCompat
import com.mydomain.android.core.DeviceIdPlan
import java.io.File
import java.util.UUID

/**
 * This phone's id for your other devices, chosen once ([DeviceIdPlan]) and
 * then kept in `noBackupFilesDir/device_id`: a backup restored onto another
 * phone never brings this phone's id with it.
 */
object DeviceId {
    private const val TAG = "DeviceId"
    private const val FILE = "device_id"

    @Synchronized
    fun resolve(ctx: Context): String {
        val app = ctx.applicationContext
        val file = File(app.noBackupFilesDir, FILE)
        val stored = runCatching { file.takeIf { it.exists() }?.readText() }.getOrNull()
        // Already kept: nothing else is read (no IMEI, no ANDROID_ID).
        if (!stored.isNullOrBlank()) return stored.trim()
        val id = DeviceIdPlan.choose(
            stored = null,
            legacyImei = legacyImei(app),
            androidId = runCatching { Settings.Secure.getString(app.contentResolver, Settings.Secure.ANDROID_ID) }.getOrNull(),
            newRandom = { UUID.randomUUID().toString() },
        )
        runCatching {
            val tmp = File(file.path + ".tmp")
            tmp.writeText(id)
            if (!tmp.renameTo(file)) {
                file.writeText(id)
                tmp.delete()
            }
        }.onFailure { Log.w(TAG, "couldn't keep the device id", it) }
        return id
    }

    /** The IMEI earlier versions used as the id: readable only on Android 9
     *  or older, and only where the permission was granted before (this
     *  version declares it up to Android 9 just so such an install keeps
     *  its id, and never asks for it). */
    @Suppress("HardwareIds", "DEPRECATION", "MissingPermission")
    private fun legacyImei(ctx: Context): String? {
        if (Build.VERSION.SDK_INT > Build.VERSION_CODES.P) return null
        if (ContextCompat.checkSelfPermission(ctx, Manifest.permission.READ_PHONE_STATE) != PackageManager.PERMISSION_GRANTED) {
            return null
        }
        val tm = ctx.getSystemService(Context.TELEPHONY_SERVICE) as? TelephonyManager ?: return null
        return runCatching { if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) tm.imei else tm.deviceId }.getOrNull()
    }
}
