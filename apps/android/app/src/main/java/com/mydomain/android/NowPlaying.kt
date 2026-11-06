package com.mydomain.android

import android.content.ComponentName
import android.content.Context
import android.graphics.Bitmap
import android.graphics.Canvas
import android.graphics.drawable.BitmapDrawable
import android.graphics.drawable.Drawable
import android.media.AudioManager
import android.media.MediaMetadata
import android.media.session.MediaController
import android.media.session.MediaSessionManager
import android.media.session.PlaybackState
import android.os.Handler
import android.os.Looper
import android.os.SystemClock
import android.util.Base64
import android.util.Log
import com.mydomain.android.core.Core
import com.mydomain.android.core.MediaPlan
import org.json.JSONArray
import org.json.JSONObject
import java.io.ByteArrayOutputStream
import java.util.concurrent.Executors
import com.mydomain.android.core.ResourceBook
import com.mydomain.android.core.ResourceLedger

/**
 * What's playing on this phone (any app with a media session: Spotify,
 * YouTube, podcasts...), handed to the core on every change so your other
 * devices can show it (core `set_now_playing`), and media commands from
 * them (`media_command` event: play, pause, next, previous, seek, volume,
 * which app to show) done on that session.
 *
 * What goes: title, artist, album, app, playing or paused, the length and
 * position, what the app lets you do (skip, seek), the volume of the
 * stream it plays on, the artwork (at most [MediaPlan.ART_PX] px, JPEG) and
 * the app icon, and the other apps that play when there are several. The
 * core sends the pictures once per watcher and throttles changes.
 *
 * Reading other apps' media sessions needs the notification listener
 * ([NotifListener]) to be enabled, the same access notification mirroring
 * uses. Without it, [start] does nothing and is tried again later (the app
 * calls it when it comes to the front, and when the listener connects).
 */
object NowPlaying {
    private const val TAG = "mydomain_media"
    private val main = Handler(Looper.getMainLooper())
    /** Core calls in order, off the main thread (a change may send). */
    private val worker = Executors.newSingleThreadExecutor()

    private var started = false
    private lateinit var app: Context
    private val watched = mutableListOf<Pair<MediaController, MediaController.Callback>>()
    private var last: String? = null
    /** The session a computer picked to show (`session` command). */
    private var chosen: String? = null
    /** The artwork already encoded, by the bitmap it was made from (held
     *  weakly: a big picture isn't kept alive for this). */
    private var artCache: Pair<java.lang.ref.WeakReference<Bitmap>, String>? = null
    /** App icons already encoded (main thread), the oldest dropped past [MAX_ICONS]. */
    private const val MAX_ICONS = 32
    private val iconCache = object : LinkedHashMap<String, String?>(16, 0.75f, true) {
        override fun removeEldestEntry(eldest: MutableMap.MutableEntry<String, String?>?): Boolean = size > MAX_ICONS
    }

    private val sessionsChanged = MediaSessionManager.OnActiveSessionsChangedListener { list ->
        track(list.orEmpty())
    }

    /** Start following media sessions (main thread or not; once). */
    fun start(ctx: Context) {
        app = ctx.applicationContext
        main.post {
            if (started) return@post
            val mgr = app.getSystemService(MediaSessionManager::class.java) ?: return@post
            val listener = ComponentName(app, NotifListener::class.java)
            try {
                mgr.addOnActiveSessionsChangedListener(sessionsChanged, listener, main)
                started = true
                ResourceLedger.hold(ResourceBook.Kind.LISTENER, "NowPlaying: media sessions")
                track(mgr.getActiveSessions(listener))
            } catch (e: SecurityException) {
                // Notification access not granted (yet).
                Log.i(TAG, "no notification access: not reading media sessions")
            }
        }
    }

    /** Follow [list] (the active sessions, most recent first). */
    private fun track(list: List<MediaController>) {
        for ((c, cb) in watched) runCatching { c.unregisterCallback(cb) }
        watched.clear()
        for (c in list) {
            val cb = object : MediaController.Callback() {
                override fun onMetadataChanged(metadata: MediaMetadata?) = push()
                override fun onPlaybackStateChanged(state: PlaybackState?) = push()
                override fun onAudioInfoChanged(info: MediaController.PlaybackInfo) = push()
                override fun onSessionDestroyed() = push()
            }
            c.registerCallback(cb, main)
            watched.add(c to cb)
        }
        push()
    }

    private fun sessionOf(c: MediaController) = MediaPlan.Session(
        id = c.packageName,
        app = appLabel(c.packageName),
        title = c.metadata?.getString(MediaMetadata.METADATA_KEY_TITLE),
        state = MediaPlan.stateWord(c.playbackState?.state ?: 0),
    )

    /** The session shown (and controlled): see [MediaPlan.pick]. */
    private fun shown(): MediaController? {
        val controllers = watched.map { it.first }
        val pick = MediaPlan.pick(controllers.map(::sessionOf), chosen) ?: return null
        return controllers.firstOrNull { it.packageName == pick.id }
    }

    /** The shown session as the core's JSON, or "". */
    private fun current(): String {
        val pick = shown() ?: return ""
        val md = pick.metadata ?: return ""
        val title = md.getString(MediaMetadata.METADATA_KEY_TITLE)
            ?: md.getString(MediaMetadata.METADATA_KEY_DISPLAY_TITLE)
            ?: return ""
        val ps = pick.playbackState
        val state = MediaPlan.stateWord(ps?.state ?: 0)
        if (state == "stopped") return ""
        val o = JSONObject()
            .put("title", title)
            .put("state", state)
            .put("app", appLabel(pick.packageName))
            .put("session", pick.packageName)
        (md.getString(MediaMetadata.METADATA_KEY_ARTIST) ?: md.getString(MediaMetadata.METADATA_KEY_ALBUM_ARTIST))
            ?.let { o.put("artist", it) }
        md.getString(MediaMetadata.METADATA_KEY_ALBUM)?.let { o.put("album", it) }
        val duration = md.getLong(MediaMetadata.METADATA_KEY_DURATION)
        if (duration > 0) o.put("duration", duration)
        if (ps != null && ps.position >= 0) {
            // The position as of now (the state says when it was true).
            val updated = ps.lastPositionUpdateTime
            val elapsed = if (updated > 0) SystemClock.elapsedRealtime() - updated else 0L
            val pos = MediaPlan.positionNow(ps.position, duration.takeIf { it > 0 }, state == "playing", elapsed)
            pos?.let { o.put("position", it) }
        }
        if (ps != null) {
            val a = ps.actions
            o.put("can_next", a and PlaybackState.ACTION_SKIP_TO_NEXT != 0L)
            o.put("can_prev", a and PlaybackState.ACTION_SKIP_TO_PREVIOUS != 0L)
            o.put("can_seek", duration > 0 && a and PlaybackState.ACTION_SEEK_TO != 0L)
        }
        pick.playbackInfo?.let { info ->
            if (info.maxVolume > 0) {
                o.put("volume", info.currentVolume).put("volume_max", info.maxVolume)
            }
        }
        art(md)?.let { o.put("art", it) }
        icon(pick.packageName)?.let { o.put("icon", it) }
        val others = watched.map { sessionOf(it.first) }.filter { it.state != "stopped" }
        if (others.size > 1) {
            val arr = JSONArray()
            for (s in others) {
                val e = JSONObject().put("id", s.id).put("app", s.app).put("state", s.state)
                s.title?.let { e.put("title", it) }
                arr.put(e)
            }
            o.put("sessions", arr)
        }
        return o.toString()
    }

    /** The track's artwork as a small base64 JPEG (kept while it's the same). */
    private fun art(md: MediaMetadata): String? {
        val bmp = md.getBitmap(MediaMetadata.METADATA_KEY_ART)
            ?: md.getBitmap(MediaMetadata.METADATA_KEY_ALBUM_ART)
            ?: md.getBitmap(MediaMetadata.METADATA_KEY_DISPLAY_ICON)
            ?: return null
        artCache?.let { (from, b64) -> if (from.get() === bmp) return b64 }
        val b64 = encode(bmp, MediaPlan.ART_PX, Bitmap.CompressFormat.JPEG, 80) ?: return null
        artCache = java.lang.ref.WeakReference(bmp) to b64
        return b64
    }

    /** The app's icon as a small base64 PNG. */
    private fun icon(pkg: String): String? = iconCache.getOrPut(pkg) {
        runCatching {
            val d = app.packageManager.getApplicationIcon(pkg)
            val b = bitmapOf(d)
            try {
                encode(b, MediaPlan.ICON_PX, Bitmap.CompressFormat.PNG, 100)
            } finally {
                // One drawn here (not the drawable's own) goes now.
                if (!(d is BitmapDrawable && d.bitmap === b)) b.recycle()
            }
        }.getOrNull()
    }

    private fun bitmapOf(d: Drawable): Bitmap {
        if (d is BitmapDrawable && d.bitmap != null) return d.bitmap
        val w = d.intrinsicWidth.coerceAtLeast(1)
        val h = d.intrinsicHeight.coerceAtLeast(1)
        val b = Bitmap.createBitmap(w, h, Bitmap.Config.ARGB_8888)
        d.setBounds(0, 0, w, h)
        d.draw(Canvas(b))
        return b
    }

    private fun encode(bmp: Bitmap, px: Int, format: Bitmap.CompressFormat, quality: Int): String? = runCatching {
        val (w, h) = MediaPlan.fit(bmp.width, bmp.height, px)
        if (w == 0) return null
        val scaled = if (w == bmp.width && h == bmp.height) bmp else Bitmap.createScaledBitmap(bmp, w, h, true)
        val out = ByteArrayOutputStream()
        try {
            scaled.compress(format, quality, out)
        } finally {
            if (scaled !== bmp) scaled.recycle()
        }
        Base64.encodeToString(out.toByteArray(), Base64.NO_WRAP)
    }.getOrNull()

    private fun appLabel(pkg: String): String = runCatching {
        val pm = app.packageManager
        pm.getApplicationLabel(pm.getApplicationInfo(pkg, 0)).toString()
    }.getOrDefault(pkg)

    private fun push() {
        val json = runCatching { current() }.getOrDefault("")
        if (json == last) return
        last = json
        worker.execute { Core.engine()?.setNowPlaying(json) }
    }

    /**
     * A computer asks (core `media_command` event, already checked: from
     * your own device): do it on the shown session, or on [session] when it
     * names one that plays here.
     */
    fun onCommand(ctx: Context, o: JSONObject) {
        if (!::app.isInitialized) app = ctx.applicationContext
        val action = o.optString("action")
        val value = if (o.isNull("value")) null else o.optLong("value")
        val session = o.optString("session").ifEmpty { null }
        main.post {
            val target = watched.map { it.first }.firstOrNull { it.packageName == session } ?: shown()
            if (target == null) {
                Log.i(TAG, "media $action: nothing plays")
                return@post
            }
            val playing = MediaPlan.stateWord(target.playbackState?.state ?: 0) == "playing"
            val op = MediaPlan.op(action, value, session, playing, target.playbackInfo?.maxVolume)
            val t = target.transportControls
            runCatching {
                when (op) {
                    MediaPlan.Op.Play -> t.play()
                    MediaPlan.Op.Pause -> t.pause()
                    MediaPlan.Op.Next -> t.skipToNext()
                    MediaPlan.Op.Previous -> t.skipToPrevious()
                    is MediaPlan.Op.SeekTo -> t.seekTo(op.ms)
                    is MediaPlan.Op.Adjust -> target.adjustVolume(
                        if (op.direction > 0) AudioManager.ADJUST_RAISE else AudioManager.ADJUST_LOWER, 0,
                    )
                    is MediaPlan.Op.SetVolume -> target.setVolumeTo(op.level, 0)
                    is MediaPlan.Op.Choose -> chosen = op.session
                    null -> Log.i(TAG, "media $action: not something it can do")
                }
            }.onFailure { Log.w(TAG, "media $action failed: $it") }
            // The volume and a picked session don't always call back.
            last = null
            main.postDelayed({ push() }, 200)
        }
    }
}
