package com.mydomain.android

import android.content.ComponentName
import android.content.Context
import android.media.MediaMetadata
import android.media.session.MediaController
import android.media.session.MediaSessionManager
import android.media.session.PlaybackState
import android.os.Handler
import android.os.Looper
import android.util.Log
import com.mydomain.android.core.Core
import org.json.JSONObject
import java.util.concurrent.Executors

/**
 * What's playing on this phone (any app with a media session: Spotify,
 * YouTube, podcasts...), handed to the core on every change so your other
 * devices can show it (core `set_now_playing`).
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
                override fun onSessionDestroyed() = push()
            }
            c.registerCallback(cb, main)
            watched.add(c to cb)
        }
        push()
    }

    /** The playing session (else a paused one) as the core's JSON, or "". */
    private fun current(): String {
        val sessions = watched.map { it.first }
        val pick = sessions.firstOrNull { it.playbackState?.state == PlaybackState.STATE_PLAYING }
            ?: sessions.firstOrNull { it.playbackState?.state == PlaybackState.STATE_PAUSED }
            ?: return ""
        val md = pick.metadata ?: return ""
        val title = md.getString(MediaMetadata.METADATA_KEY_TITLE)
            ?: md.getString(MediaMetadata.METADATA_KEY_DISPLAY_TITLE)
            ?: return ""
        val o = JSONObject()
            .put("title", title)
            .put("state", if (pick.playbackState?.state == PlaybackState.STATE_PLAYING) "playing" else "paused")
            .put("app", appLabel(pick.packageName))
        (md.getString(MediaMetadata.METADATA_KEY_ARTIST) ?: md.getString(MediaMetadata.METADATA_KEY_ALBUM_ARTIST))
            ?.let { o.put("artist", it) }
        md.getString(MediaMetadata.METADATA_KEY_ALBUM)?.let { o.put("album", it) }
        return o.toString()
    }

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
}
