package com.mydomain.android.core

/**
 * Media controls, the rules without Android (core `features::media`; JVM
 * tested): which of this phone's media sessions is shown and controlled,
 * how a command from a computer maps onto it, and how the other side's
 * position runs on between updates.
 */
object MediaPlan {
    /** What a computer may ask (the core already checked it). */
    val ACTIONS = setOf(
        "play", "pause", "play_pause", "next", "previous", "seek",
        "volume_up", "volume_down", "volume_set", "session",
    )

    /** Longest artwork side sent, px (a 256 px JPEG stays well under the
     *  core's limit). */
    const val ART_PX = 256
    /** Longest app icon side sent, px. */
    const val ICON_PX = 64
    /** How often a watching screen asks to stay posted (the core keeps a
     *  watcher for ten minutes). */
    const val WATCH_EVERY_MS = 4 * 60_000L

    /** One media session as the rules see it. */
    data class Session(val id: String, val app: String, val title: String?, val state: String)

    /**
     * The session to show: the one the user picked (from a computer) while
     * it's still there, else the one playing, else a paused one, else none.
     */
    fun pick(sessions: List<Session>, chosen: String?): Session? =
        sessions.firstOrNull { it.id == chosen }
            ?: sessions.firstOrNull { it.state == "playing" }
            ?: sessions.firstOrNull { it.state == "paused" }

    /** What a command does to a session (Android's MediaController). */
    sealed class Op {
        object Play : Op()
        object Pause : Op()
        object Next : Op()
        object Previous : Op()
        data class SeekTo(val ms: Long) : Op()
        /** Volume up (+1) or down (-1) a step, on the session's stream. */
        data class Adjust(val direction: Int) : Op()
        data class SetVolume(val level: Int) : Op()
        data class Choose(val session: String) : Op()
    }

    /**
     * [action] with [value] / [session] as an [Op], given whether the shown
     * session [playing] now and the session's [volumeMax]; null for one it
     * can't do. Play/pause toggles on what's true now, not on what the
     * computer last saw.
     */
    fun op(action: String, value: Long?, session: String?, playing: Boolean, volumeMax: Int?): Op? = when (action) {
        "play" -> Op.Play
        "pause" -> Op.Pause
        "play_pause" -> if (playing) Op.Pause else Op.Play
        "next" -> Op.Next
        "previous" -> Op.Previous
        "seek" -> value?.takeIf { it >= 0 }?.let { Op.SeekTo(it) }
        "volume_up" -> Op.Adjust(1)
        "volume_down" -> Op.Adjust(-1)
        "volume_set" -> value?.let { v -> volumeMax?.takeIf { it > 0 }?.let { Op.SetVolume(v.coerceIn(0, it.toLong()).toInt()) } }
        "session" -> session?.takeIf { it.isNotBlank() }?.let { Op.Choose(it) }
        else -> null
    }

    /**
     * Where a playing track is [elapsedMs] after it was at [position]
     * (never past [duration]; a paused one stays).
     */
    fun positionNow(position: Long?, duration: Long?, playing: Boolean, elapsedMs: Long): Long? {
        val p = position ?: return null
        if (!playing) return p
        val moved = p + elapsedMs.coerceAtLeast(0)
        return if (duration != null && duration > 0) moved.coerceAtMost(duration) else moved
    }

    /** "3:07", "1:02:05" for a time in ms. */
    fun clock(ms: Long): String {
        val s = (ms.coerceAtLeast(0) / 1000)
        val h = s / 3600
        val m = (s % 3600) / 60
        val sec = s % 60
        return if (h > 0) "%d:%02d:%02d".format(h, m, sec) else "%d:%02d".format(m, sec)
    }

    /** The seek target for a slider at [fraction] (0..1) of [duration]. */
    fun seekTarget(fraction: Float, duration: Long): Long =
        (fraction.coerceIn(0f, 1f) * duration).toLong().coerceIn(0, duration)

    /** Android's playback state codes into the core's word. */
    fun stateWord(code: Int): String = when (code) {
        3 /* STATE_PLAYING */, 6 /* BUFFERING */, 4 /* FAST_FORWARDING */, 5 /* REWINDING */ -> "playing"
        2 /* STATE_PAUSED */ -> "paused"
        else -> "stopped"
    }

    /** Fit a [w]×[h] picture inside [max] px on its longest side (never
     *  bigger than it is). */
    fun fit(w: Int, h: Int, max: Int): Pair<Int, Int> {
        if (w <= 0 || h <= 0) return 0 to 0
        val scale = minOf(1.0, max.toDouble() / maxOf(w, h))
        return maxOf(1, (w * scale).toInt()) to maxOf(1, (h * scale).toInt())
    }
}
