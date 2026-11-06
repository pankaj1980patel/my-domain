package com.mydomain.android

import android.content.Context
import android.content.Intent
import android.graphics.Bitmap
import android.graphics.Canvas
import android.graphics.Color
import android.graphics.DashPathEffect
import android.graphics.Paint
import android.graphics.Typeface
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.os.SystemClock
import android.util.Log
import androidx.core.content.pm.ShortcutInfoCompat
import androidx.core.content.pm.ShortcutManagerCompat
import androidx.core.graphics.PathParser
import androidx.core.graphics.drawable.IconCompat
import com.mydomain.android.core.Core
import com.mydomain.android.core.DeviceIconShapes
import com.mydomain.android.core.DeviceKinds
import com.mydomain.android.core.DeviceLabels
import com.mydomain.android.core.initial
import java.util.concurrent.Executors

/**
 * Direct Share: each of your devices is its own target in Android's share
 * sheet ("Pankaj's MacBook Pro"), so a share goes straight there without the
 * app's device picker ([DirectShareActivity] sends it).
 *
 * Sharing shortcuts (Android 10+): one dynamic shortcut per device, id = its
 * node id, named by [Core.peerName] (so nicknames show), in the
 * [CATEGORY] that `res/xml/shortcuts.xml` declares as a share target for
 * MainActivity. Devices with a live link rank first. [Core] calls [refresh]
 * when the device list, a link or a label changes; devices that are gone
 * lose their shortcut. The same shortcuts show on a long press of the app's
 * launcher icon, where they open that device's screen ([EXTRA_OPEN_DEVICE]).
 *
 * Older Android has no sharing shortcuts without another library, so there
 * the share sheet shows the app alone, as before.
 */
object ShareTargets {
    /** Must match the `<category>` in `res/xml/shortcuts.xml`. */
    const val CATEGORY = "com.mydomain.android.category.SHARE_TARGET"

    /** On MainActivity's intent: open this node's device screen. */
    const val EXTRA_OPEN_DEVICE = "com.mydomain.android.OPEN_DEVICE"

    /** Static shortcuts in `res/xml/shortcuts.xml` ("New note"): they count
     *  towards Android's per-activity limit too. */
    const val STATIC_SHORTCUTS = 1

    private const val TAG = "ShareTargets"
    private const val GAP_MS = 2_000L

    private val main = Handler(Looper.getMainLooper())
    private val worker = Executors.newSingleThreadExecutor()
    private var last = 0L
    private var pending = false

    /** What was last pushed (ids, names, icons, order), to skip repeats. */
    @Volatile private var pushed: List<String>? = null

    fun supported(): Boolean = Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q

    /** Bring the shortcuts up to date soon (at most once per [GAP_MS]; the
     *  last call always lands). Safe from any thread. */
    fun refresh(context: Context) {
        if (!supported()) return
        val app = context.applicationContext
        main.post {
            if (pending) return@post
            val wait = last + GAP_MS - SystemClock.elapsedRealtime()
            if (wait <= 0) {
                update(app)
            } else {
                pending = true
                main.postDelayed({ pending = false; update(app) }, wait)
            }
        }
    }

    private data class Target(val node: String, val name: String, val icon: String?, val color: String?, val linked: Boolean)

    /** Main thread: read the devices, then push off it. */
    private fun update(app: Context) {
        last = SystemClock.elapsedRealtime()
        // No engine (yet): the device list is unknown, not empty; keep
        // what's there rather than removing every shortcut.
        if (Core.engine() == null) return
        val targets = Core.peers
            .map { p ->
                val label = DeviceLabels.labels[p.nodeId]
                Target(p.nodeId, Core.peerName(p.nodeId), DeviceKinds.iconOf(p.nodeId), label?.color, Core.links.containsKey(p.nodeId))
            }
            .distinctBy { it.node }
            .sortedWith(compareByDescending<Target> { it.linked }.thenBy { it.name.lowercase() })
        worker.execute { push(app, targets) }
    }

    private fun push(app: Context, all: List<Target>) {
        runCatching {
            val max = (ShortcutManagerCompat.getMaxShortcutCountPerActivity(app) - STATIC_SHORTCUTS).coerceAtLeast(1)
            val targets = all.take(max)
            val signature = targets.map { "${it.node}|${it.name}|${it.icon}|${it.color}" }
            if (signature == pushed) return
            val keep = targets.map { it.node }.toSet()
            val stale = ShortcutManagerCompat.getDynamicShortcuts(app).map { it.id }.filter { it !in keep }
            if (stale.isNotEmpty()) ShortcutManagerCompat.removeDynamicShortcuts(app, stale)
            targets.forEachIndexed { rank, t -> ShortcutManagerCompat.pushDynamicShortcut(app, shortcut(app, t, rank)) }
            pushed = signature
        }.onFailure { Log.w(TAG, "couldn't update share targets", it) }
    }

    private fun shortcut(app: Context, t: Target, rank: Int): ShortcutInfoCompat {
        // A launcher long press opens the device's screen.
        val open = Intent(app, MainActivity::class.java)
            .setAction(Intent.ACTION_VIEW)
            .putExtra(EXTRA_OPEN_DEVICE, t.node)
        return ShortcutInfoCompat.Builder(app, t.node)
            .setShortLabel(t.name)
            .setLongLabel(t.name)
            .setIcon(IconCompat.createWithAdaptiveBitmap(iconBitmap(app, t)))
            .setIntent(open)
            .setCategories(setOf(CATEGORY))
            .setRank(rank)
            .build()
    }

    /** The label's accent colours as plain ARGB (the Cosmic Blue theme's,
     *  as for the widget: a shortcut can't use the Compose theme). */
    private fun colorOf(id: String?): Int = when (id) {
        "cyan" -> 0xFF22C7F2.toInt()
        "blue" -> 0xFF1455D9.toInt()
        "sky" -> 0xFF64BFFF.toInt()
        "green" -> 0xFF34D399.toInt()
        "gold" -> 0xFFFFC857.toInt()
        "rose" -> 0xFFFB7185.toInt()
        "grey" -> 0xFF94A3B8.toInt()
        else -> 0xFF22C7F2.toInt()
    }

    /**
     * An adaptive icon (108dp, its middle 66dp always shown): the device's
     * icon (the one you chose, else the one for what it is; the same line
     * drawings as DeviceIcon), in its colour on the app's deep blue.
     */
    private fun iconBitmap(app: Context, t: Target): Bitmap {
        val density = app.resources.displayMetrics.density
        val px = (108 * density).toInt().coerceIn(108, 432)
        val bmp = Bitmap.createBitmap(px, px, Bitmap.Config.ARGB_8888)
        val c = Canvas(bmp)
        c.drawColor(0xFF0B1630.toInt())
        val color = colorOf(t.color)
        val dp = px / 108f
        // A ring inside the safe zone, in the device's colour.
        val ring = Paint(Paint.ANTI_ALIAS_FLAG).apply {
            style = Paint.Style.STROKE
            strokeWidth = 2.5f * dp
            this.color = color
        }
        c.drawCircle(px / 2f, px / 2f, 30f * dp, ring)
        val glyph = t.icon
        if (glyph != null && glyph != "unknown" && glyph in DeviceLabels.ICONS) {
            drawDeviceIcon(c, glyph, px / 2f, px / 2f, 36f * dp, Color.WHITE)
        } else {
            val text = Paint(Paint.ANTI_ALIAS_FLAG).apply {
                this.color = Color.WHITE
                textSize = 30f * dp
                typeface = Typeface.create(Typeface.DEFAULT, Typeface.BOLD)
                textAlign = Paint.Align.CENTER
            }
            val letter = initial(t.name)
            val y = px / 2f - (text.descent() + text.ascent()) / 2f
            c.drawText(letter, px / 2f, y, text)
        }
        return bmp
    }

    /** ui/DeviceLabelUi.kt's DeviceIcon on an android.graphics.Canvas: the
     *  same 24-unit drawing ([DeviceIconShapes]), [size] px wide, centred
     *  on ([cx], [cy]). */
    private fun drawDeviceIcon(c: Canvas, icon: String, cx: Float, cy: Float, size: Float, color: Int) {
        val stroke = Paint(Paint.ANTI_ALIAS_FLAG).apply {
            style = Paint.Style.STROKE
            strokeCap = Paint.Cap.ROUND
            strokeJoin = Paint.Join.ROUND
            this.color = color
        }
        val fill = Paint(Paint.ANTI_ALIAS_FLAG).apply {
            style = Paint.Style.FILL
            this.color = color
        }
        val dash = DashPathEffect(floatArrayOf(1.6f, 1.6f), 0f)
        fun draw(parts: List<DeviceIconShapes.Part>) {
            for (part in parts) {
                val path = runCatching { PathParser.createPathFromPathData(part.d) }.getOrNull() ?: continue
                if (part.fill) {
                    c.drawPath(path, fill)
                } else {
                    stroke.strokeWidth = if (part.thin) 0.9f else 1.8f
                    stroke.pathEffect = if (part.dashed) dash else null
                    c.drawPath(path, stroke)
                }
            }
        }
        val shape = DeviceIconShapes.shape(icon)
        c.save()
        c.translate(cx - size / 2f, cy - size / 2f)
        c.scale(size / 24f, size / 24f)
        draw(shape.parts)
        shape.mark?.let { m ->
            c.save()
            c.translate(m.x, m.y)
            c.scale(m.scale, m.scale)
            draw(DeviceIconShapes.markParts(m.kind))
            c.restore()
        }
        c.restore()
    }

    /** A share through one of these shortcuts: helps Android rank it. */
    fun reportUsed(context: Context, node: String) {
        if (!supported()) return
        runCatching { ShortcutManagerCompat.reportShortcutUsed(context.applicationContext, node) }
    }

    /** The name a shortcut was given, for when the device list isn't
     *  loaded yet (a share right after the app started). */
    fun labelOf(context: Context, node: String): String? = runCatching {
        ShortcutManagerCompat.getDynamicShortcuts(context.applicationContext)
            .firstOrNull { it.id == node }?.shortLabel?.toString()
    }.getOrNull()
}
