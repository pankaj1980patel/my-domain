package com.mydomain.android

import com.mydomain.android.core.AppScope
import android.app.WallpaperManager
import android.content.Context
import android.graphics.Bitmap
import android.graphics.Canvas
import android.graphics.drawable.Drawable
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.util.Log
import com.mydomain.android.core.Core
import java.io.ByteArrayOutputStream

/**
 * "Share my wallpaper with my devices" (the core's `features::wallpaper`),
 * on by default: your computers draw this phone's tile from its wallpaper,
 * the way the phone shows its lock screen.
 *
 * - Always its colours: [WallpaperManager.getWallpaperColors] for the lock
 *   screen, else the home screen (Android 8.1 and later; no permission).
 * - The picture only when Android hands it over without asking for
 *   anything new ([WallpaperManager.getDrawable] in a try/catch): never a
 *   permission request, never All files access. On Android 13 and later it
 *   throws, or gives the stock wallpaper instead of yours (this app targets
 *   an older Android); a picture that is the stock one is left out there,
 *   so a Pixel on Android 14+ shares its colours only. A live wallpaper
 *   shares its colours only (its still isn't what shows).
 * - Sent when the colours change (Android says so) and at start; the core
 *   sends only a look whose hash changed. Off: nothing is read or sent,
 *   and your computers forget the one they had.
 */
object PhoneWallpaper {
    const val PREF = "share_wallpaper"
    private const val TAG = "PhoneWallpaper"
    /** The picture's long side, in pixels. */
    private const val SIDE = 320
    /** The picture's most bytes. */
    private const val MAX_BYTES = 40 * 1024

    @Volatile private var listening = false

    fun enabled(ctx: Context): Boolean =
        ctx.getSharedPreferences("md", Context.MODE_PRIVATE).getBoolean(PREF, true)

    fun set(ctx: Context, on: Boolean) {
        ctx.getSharedPreferences("md", Context.MODE_PRIVATE).edit().putBoolean(PREF, on).apply()
        report(ctx)
    }

    /** Say it now, and again whenever the wallpaper's colours change. */
    fun start(ctx: Context) {
        val app = ctx.applicationContext
        report(app)
        if (Build.VERSION.SDK_INT >= 27 && !listening) {
            listening = true
            runCatching {
                WallpaperManager.getInstance(app).addOnColorsChangedListener(
                    { _, _ -> report(app) },
                    Handler(Looper.getMainLooper()),
                )
            }.onFailure { Log.w(TAG, "can't follow the wallpaper", it) }
        }
    }

    /** Hand the core this phone's look (or "nothing", when off). */
    fun report(ctx: Context) {
        val app = ctx.applicationContext
        val engine = Core.engine() ?: return
        if (!enabled(app)) {
            runCatching { engine.clearWallpaper() }
            return
        }
        AppScope.bg("wallpaper") {
            val colors = colors(app)
            val image = image(app)
            runCatching {
                if (colors.isEmpty() && image == null) engine.clearWallpaper() else engine.setWallpaper(colors, image)
            }.onFailure { Log.w(TAG, "can't share the wallpaper", it) }
        }
    }

    /** Primary, secondary and tertiary colours, `#rrggbb`. */
    private fun colors(ctx: Context): List<String> {
        if (Build.VERSION.SDK_INT < 27) return emptyList()
        return runCatching {
            val wm = WallpaperManager.getInstance(ctx)
            val c = wm.getWallpaperColors(WallpaperManager.FLAG_LOCK)
                ?: wm.getWallpaperColors(WallpaperManager.FLAG_SYSTEM)
                ?: return emptyList()
            listOfNotNull(c.primaryColor, c.secondaryColor, c.tertiaryColor)
                .map { String.format("#%06x", it.toArgb() and 0xffffff) }
                .distinct()
        }.getOrDefault(emptyList())
    }

    /** The picture as a small JPEG, only if Android hands it over as is. */
    private fun image(ctx: Context): ByteArray? = try {
        val wm = WallpaperManager.getInstance(ctx)
        if (wm.wallpaperInfo != null) {
            null
        } else {
            val d = if (Build.VERSION.SDK_INT >= 34) {
                wm.getDrawable(WallpaperManager.FLAG_LOCK) ?: wm.getDrawable(WallpaperManager.FLAG_SYSTEM)
            } else {
                @Suppress("DEPRECATION")
                wm.drawable
            }
            d?.let { drawable ->
                // Android 13+: a picture we may not read comes back as the
                // stock one; that isn't this phone's.
                if (Build.VERSION.SDK_INT >= 33 && sameAs(drawable, runCatching { wm.builtInDrawable }.getOrNull())) {
                    null
                } else {
                    jpeg(drawable)
                }
            }
        }
    } catch (e: SecurityException) {
        null
    } catch (e: Throwable) {
        Log.w(TAG, "can't read the wallpaper", e)
        null
    }

    /** [d] drawn at most [w] × [h], keeping its shape. */
    private fun bitmapOf(d: Drawable, w: Int, h: Int): Bitmap? {
        val iw = d.intrinsicWidth
        val ih = d.intrinsicHeight
        if (iw <= 0 || ih <= 0) return null
        val scale = minOf(w.toFloat() / iw, h.toFloat() / ih, 1f)
        val bw = maxOf(1, (iw * scale).toInt())
        val bh = maxOf(1, (ih * scale).toInt())
        val bmp = Bitmap.createBitmap(bw, bh, Bitmap.Config.ARGB_8888)
        val old = d.copyBounds()
        d.setBounds(0, 0, bw, bh)
        d.draw(Canvas(bmp))
        d.bounds = old
        return bmp
    }

    private fun jpeg(d: Drawable): ByteArray? {
        val bmp = bitmapOf(d, SIDE, SIDE) ?: return null
        try {
            for (q in intArrayOf(80, 65, 50, 36, 26)) {
                val out = ByteArrayOutputStream()
                bmp.compress(Bitmap.CompressFormat.JPEG, q, out)
                if (out.size() <= MAX_BYTES) return out.toByteArray()
            }
            return null
        } finally {
            bmp.recycle()
        }
    }

    /** Whether two pictures look the same (16 × 16, a little noise allowed). */
    private fun sameAs(a: Drawable, b: Drawable?): Boolean {
        if (b == null) return false
        val x = bitmapOf(a, 16, 16)?.let { Bitmap.createScaledBitmap(it, 16, 16, true) } ?: return false
        val y = bitmapOf(b, 16, 16)?.let { Bitmap.createScaledBitmap(it, 16, 16, true) } ?: return false
        val px = IntArray(256).also { x.getPixels(it, 0, 16, 0, 0, 16, 16) }
        val py = IntArray(256).also { y.getPixels(it, 0, 16, 0, 0, 16, 16) }
        var diff = 0L
        for (i in 0 until 256) {
            for (shift in intArrayOf(16, 8, 0)) {
                diff += kotlin.math.abs(((px[i] shr shift) and 0xff) - ((py[i] shr shift) and 0xff))
            }
        }
        return diff / (256 * 3) < 6
    }
}
