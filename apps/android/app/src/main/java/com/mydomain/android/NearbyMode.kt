package com.mydomain.android

import android.content.Context
import com.mydomain.android.core.Core

/**
 * The switch "Use nearby mode for media" (Settings; on by default): while
 * audio, a screen or the camera streams with a device that's near, the core
 * moves the stream to a direct link on the local network (mdcore
 * `features::nearby`). Kept in the "md" preferences (so it's in the
 * settings backup) and handed to the core, which keeps its own copy.
 */
object NearbyMode {
    const val PREF = "nearby_media"

    fun enabled(ctx: Context): Boolean =
        ctx.getSharedPreferences("md", Context.MODE_PRIVATE).getBoolean(PREF, true)

    fun set(ctx: Context, on: Boolean) {
        ctx.getSharedPreferences("md", Context.MODE_PRIVATE).edit().putBoolean(PREF, on).apply()
        Core.engine()?.setNearbyMedia(on)
    }

    /** Hand the preference to the core (at start, and after a restore). */
    fun apply(ctx: Context) {
        Core.engine()?.setNearbyMedia(enabled(ctx))
    }
}
