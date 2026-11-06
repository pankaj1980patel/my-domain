package com.mydomain.android.core

import android.content.Context
import android.content.pm.PackageManager
import android.os.Build
import android.provider.Settings
import android.util.Log
import androidx.compose.runtime.mutableStateMapOf
import com.mydomain.android.ffi.MdEngine
import org.json.JSONObject

/**
 * What kind of device each of your devices is (the core's
 * `features::device_kind`), so screens draw the right icon: a Mac laptop,
 * a Windows desktop, an Android tablet... This phone works out its own
 * kind at start ([reportOwn]) and hands it to the core, which says it to
 * your other devices; theirs come from the core ([load], and the
 * `device_kind` event, [apply]). The icon you choose for a device (its
 * label, [DeviceLabels]) wins over this one ([iconOf]).
 */
object DeviceKinds {
    /** node id -> its kind (changed on the main thread). */
    val kinds = mutableStateMapOf<String, DeviceKind>()

    /** This phone's own kind, once [reportOwn] ran. */
    @Volatile var own: DeviceKind? = null
        private set

    /** The icon drawn for [node]: the one you chose, else its own. */
    fun iconOf(node: String): String = DeviceKindPlan.iconOf(DeviceLabels.labels[node]?.icon, kinds[node])

    /** "Apple MacBook Pro" for [node]; "" when it never said. */
    fun describe(node: String): String = kinds[node]?.description.orEmpty()

    fun parse(o: JSONObject?): DeviceKind? = o?.let {
        DeviceKind(
            os = it.optString("os"),
            form = it.optString("form"),
            model = it.optString("model"),
            maker = it.optString("maker"),
            icon = it.optString("icon").ifEmpty { "unknown" },
        )
    }

    /** Every kind the core knows (after start). Any thread. */
    fun load(engine: MdEngine) {
        val json = runCatching { JSONObject(engine.deviceKindsJson()) }.getOrNull() ?: return
        val all = HashMap<String, DeviceKind>()
        for (node in json.keys()) parse(json.optJSONObject(node))?.let { all[node] = it }
        Core.onMain {
            kinds.clear()
            kinds.putAll(all)
        }
    }

    /** A `device_kind` event: `{node, device}`. Main thread. */
    fun apply(o: JSONObject) {
        val node = o.optString("node").ifEmpty { return }
        parse(o.optJSONObject("device"))?.let { kinds[node] = it }
    }

    /** Work out this phone's kind and hand it to the core. */
    fun reportOwn(ctx: Context, engine: MdEngine) {
        val pm = ctx.packageManager
        val hinge = Build.VERSION.SDK_INT >= 30 && runCatching { pm.hasSystemFeature(PackageManager.FEATURE_SENSOR_HINGE_ANGLE) }.getOrDefault(false)
        @Suppress("DEPRECATION")
        val tv = runCatching { pm.hasSystemFeature(PackageManager.FEATURE_LEANBACK) }.getOrDefault(false)
        val sw = ctx.resources.configuration.smallestScreenWidthDp
        val deviceName = if (Build.VERSION.SDK_INT >= 25) {
            runCatching { Settings.Global.getString(ctx.contentResolver, Settings.Global.DEVICE_NAME) }.getOrNull()
        } else {
            null
        }
        val maker = DeviceKindPlan.tidyMaker(Build.MANUFACTURER.orEmpty())
        val kind = DeviceKind(
            os = "android",
            form = DeviceKindPlan.form(sw, hinge, tv),
            model = DeviceKindPlan.displayModel(maker, Build.MODEL.orEmpty(), deviceName),
            maker = maker,
        ).let { it.copy(icon = DeviceKindPlan.androidIcon(it.form)) }
        own = kind
        runCatching { engine.setDeviceKind(kind.os, kind.form, kind.model, kind.maker) }
            .onFailure { Log.w("DeviceKinds", "can't say this phone's kind", it) }
        load(engine)
    }
}
