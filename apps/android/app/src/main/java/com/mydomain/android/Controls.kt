package com.mydomain.android

import android.content.Context
import android.content.pm.PackageManager
import android.hardware.camera2.CameraCharacteristics
import android.hardware.camera2.CameraManager
import android.media.AudioManager
import android.os.Handler
import android.os.Looper
import android.os.SystemClock
import android.util.Log
import android.view.KeyEvent
import org.json.JSONArray
import org.json.JSONObject

/**
 * Actions your other devices can run on this phone (core `action` events):
 *
 *   flashlight {on: bool}
 *   volume     {step: 1 | -1}            media volume one notch
 *   media      {key: "play_pause" | "next" | "previous"}
 *   ringer     {mode: "normal" | "vibrate" | "silent"}
 *   launch     {pkg}                     open an app (from the apps list)
 *   dial       {number}                  open the dialer with it (no call)
 *
 * [actions] lists the ones this phone supports and [state] what they're set
 * to now; both go into the status other devices ask for, so they only show
 * buttons that work. [onChange] is called when the state may have changed.
 */
object Controls {
    private const val TAG = "Controls"
    private var torchOn = false
    private var torchId: String? = null
    var onChange: () -> Unit = {}

    private var hasPhone = false

    fun init(ctx: Context) {
        hasPhone = ctx.packageManager.hasSystemFeature(PackageManager.FEATURE_TELEPHONY)
        if (!ctx.packageManager.hasSystemFeature(PackageManager.FEATURE_CAMERA_FLASH)) return
        val cm = ctx.getSystemService(CameraManager::class.java) ?: return
        torchId = runCatching {
            cm.cameraIdList.firstOrNull { cm.getCameraCharacteristics(it).get(CameraCharacteristics.FLASH_INFO_AVAILABLE) == true }
        }.getOrNull()
        if (torchId == null) return
        cm.registerTorchCallback(object : CameraManager.TorchCallback() {
            override fun onTorchModeChanged(cameraId: String, enabled: Boolean) {
                if (cameraId == torchId && enabled != torchOn) {
                    torchOn = enabled
                    onChange()
                }
            }
        }, Handler(Looper.getMainLooper()))
    }

    fun actions(): JSONArray = JSONArray().apply {
        if (torchId != null) put("flashlight")
        put("volume"); put("media"); put("ringer"); put("launch")
        if (hasPhone) put("dial")
    }

    fun state(ctx: Context): JSONObject {
        val am = ctx.getSystemService(AudioManager::class.java)
        val o = JSONObject()
        if (torchId != null) o.put("torch", torchOn)
        if (am != null) {
            val max = am.getStreamMaxVolume(AudioManager.STREAM_MUSIC).coerceAtLeast(1)
            o.put("volume", am.getStreamVolume(AudioManager.STREAM_MUSIC) * 100 / max)
            o.put("ringer", when (am.ringerMode) {
                AudioManager.RINGER_MODE_SILENT -> "silent"
                AudioManager.RINGER_MODE_VIBRATE -> "vibrate"
                else -> "normal"
            })
        }
        return o
    }

    /** Run action [name]; returns false if it's unknown or couldn't be done. */
    fun run(ctx: Context, name: String, args: JSONObject, fromName: String = ""): Boolean {
        val am = ctx.getSystemService(AudioManager::class.java)
        val ok = runCatching {
            when (name) {
                "flashlight" -> {
                    val id = torchId ?: return false
                    ctx.getSystemService(CameraManager::class.java)?.setTorchMode(id, args.optBoolean("on", !torchOn))
                    true
                }
                "volume" -> {
                    val dir = if (args.optInt("step", 1) < 0) AudioManager.ADJUST_LOWER else AudioManager.ADJUST_RAISE
                    am?.adjustStreamVolume(AudioManager.STREAM_MUSIC, dir, 0)
                    am != null
                }
                "media" -> {
                    val code = when (args.optString("key")) {
                        "play_pause" -> KeyEvent.KEYCODE_MEDIA_PLAY_PAUSE
                        "next" -> KeyEvent.KEYCODE_MEDIA_NEXT
                        "previous" -> KeyEvent.KEYCODE_MEDIA_PREVIOUS
                        else -> return false
                    }
                    val t = SystemClock.uptimeMillis()
                    am?.dispatchMediaKeyEvent(KeyEvent(t, t, KeyEvent.ACTION_DOWN, code, 0))
                    am?.dispatchMediaKeyEvent(KeyEvent(t, t, KeyEvent.ACTION_UP, code, 0))
                    am != null
                }
                "ringer" -> {
                    val mode = when (args.optString("mode")) {
                        "silent" -> AudioManager.RINGER_MODE_SILENT
                        "vibrate" -> AudioManager.RINGER_MODE_VIBRATE
                        "normal" -> AudioManager.RINGER_MODE_NORMAL
                        else -> return false
                    }
                    // Silent needs Do Not Disturb access on Android 7+;
                    // without it, vibrate is the closest we may set.
                    try {
                        am?.ringerMode = mode
                    } catch (e: SecurityException) {
                        am?.ringerMode = AudioManager.RINGER_MODE_VIBRATE
                    }
                    am != null
                }
                "launch" -> DeviceActions.launchApp(ctx, fromName, args.optString("pkg"))
                "dial" -> DeviceActions.dial(ctx, fromName, args.optString("number"))
                else -> false
            }
        }.onFailure { Log.w(TAG, "action $name failed", it) }.getOrDefault(false)
        onChange()
        return ok
    }
}
