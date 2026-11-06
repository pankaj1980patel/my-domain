package com.mydomain.android.core

/**
 * When timers ring on this phone, and how they read (the pure part of
 * `TimerAlarms`; the desktop's `timers.rs` follows the same rules).
 *
 * A timer rings here when it ends if this phone is one of the devices it
 * rings on. It rings once per end time: what rang (timer id -> its end) is
 * kept, so a restart or a second alarm doesn't ring it again, while a
 * timer given more time (or started again) rings again at its new end. One
 * that ended over [LATE_MS] ago (the phone was off) doesn't ring; it shows
 * as ended until it's stopped.
 */
object TimerPlan {
    /** A timer that ended this long ago doesn't ring any more. */
    const val LATE_MS = 10L * 60 * 1000
    /** The longest timer the core takes (a week). */
    const val MAX_MS = 7L * 24 * 3600 * 1000
    /** The quick ones, in minutes. */
    val PRESETS = listOf(1, 3, 5, 10, 15, 30, 60)

    /** One timer, as kept for the alarm (the app may not be running when
     *  it goes off). */
    data class Entry(
        val id: String,
        val label: String,
        val state: String,
        val endsAt: Long,
        val remainingMs: Long,
        val durationMs: Long,
        val ringOn: List<String>,
    )

    /** It ended (and hasn't been stopped): it rings where it rings. */
    fun ended(state: String, endsAt: Long, now: Long) = state == "running" && endsAt <= now

    /** What's left at [now], in ms. */
    fun remaining(state: String, endsAt: Long, remainingMs: Long, now: Long): Long =
        if (state == "paused") remainingMs else maxOf(0L, endsAt - now)

    /** How far along it is, 0..1. */
    fun progress(state: String, endsAt: Long, remainingMs: Long, durationMs: Long, now: Long): Float {
        if (durationMs <= 0) return 1f
        val left = remaining(state, endsAt, remainingMs, now)
        return (1f - left.toFloat() / durationMs).coerceIn(0f, 1f)
    }

    /** "1:05:03", "4:32", "0:07": hours only when there are some; seconds
     *  rounded up, as a timer shows them. */
    fun clock(ms: Long): String {
        val total = (maxOf(0L, ms) + 999) / 1000
        val h = total / 3600
        val m = (total / 60) % 60
        val s = total % 60
        return if (h > 0) "%d:%02d:%02d".format(h, m, s) else "%d:%02d".format(m, s)
    }

    /** Hours, minutes and seconds typed in, as ms (null: not numbers, under
     *  a second, or over a week). */
    fun durationOf(h: String, m: String, s: String): Long? {
        fun num(v: String): Long? = if (v.isBlank()) 0L else v.trim().toLongOrNull()?.takeIf { it >= 0 && it < 1_000_000 }
        val hh = num(h) ?: return null
        val mm = num(m) ?: return null
        val ss = num(s) ?: return null
        val ms = (hh * 3600 + mm * 60 + ss) * 1000
        return ms.takeIf { it in 1000..MAX_MS }
    }

    /**
     * A look at every timer at [now] for this phone ([me]): [ring] these,
     * stop ringing [quiet] (rang before, now stopped, given more time,
     * paused or gone), set the alarm for [next], and keep [rung].
     */
    data class Plan(val ring: List<String>, val quiet: List<String>, val next: Long?, val rung: Map<String, Long>)

    fun plan(entries: Collection<Entry>, me: String, rung: Map<String, Long>, now: Long): Plan {
        val ring = mutableListOf<String>()
        val kept = HashMap<String, Long>()
        var next: Long? = null
        for (e in entries) {
            if (e.state != "running" || me !in e.ringOn) continue
            if (e.endsAt > now) {
                next = next?.let { minOf(it, e.endsAt) } ?: e.endsAt
                continue
            }
            if (rung[e.id] != e.endsAt && now - e.endsAt < LATE_MS) ring.add(e.id)
            kept[e.id] = e.endsAt
        }
        val quiet = rung.keys.filter { it !in kept }
        return Plan(ring, quiet, next, kept)
    }

    private const val MINUTE = 60_000L

    /** The timer kept for the alarm, as a notification's button ([op]:
     *  "dismiss", "cancel", "pause", "resume" or "add", a minute) leaves it
     *  at [now], before the core hears of it (null: gone). */
    fun applied(e: Entry, op: String, now: Long): Entry? = when (op) {
        "dismiss", "cancel" -> null
        "pause" -> if (e.state == "running" && e.endsAt > now) e.copy(state = "paused", remainingMs = e.endsAt - now, endsAt = 0) else e
        "resume" -> if (e.state == "paused") e.copy(state = "running", endsAt = now + e.remainingMs, remainingMs = 0) else e
        "add" -> when {
            ended(e.state, e.endsAt, now) -> e.copy(endsAt = now + MINUTE, durationMs = MINUTE)
            e.state == "running" -> e.copy(endsAt = e.endsAt + MINUTE, durationMs = e.durationMs + MINUTE)
            else -> e.copy(remainingMs = e.remainingMs + MINUTE, durationMs = e.durationMs + MINUTE)
        }
        else -> e
    }

    /** Running (not ended) and paused ones: shown in the ongoing
     *  notification, soonest end first, then paused. */
    fun ongoing(entries: Collection<Entry>, now: Long): List<Entry> =
        entries.filter { it.state == "paused" || (it.state == "running" && it.endsAt > now) }
            .sortedWith(compareBy<Entry> { if (it.state == "running") 0 else 1 }.thenBy { it.endsAt }.thenBy { it.id })
}
