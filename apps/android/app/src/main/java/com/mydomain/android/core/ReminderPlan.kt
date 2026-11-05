package com.mydomain.android.core

import java.util.Calendar
import java.util.TimeZone

/**
 * When note reminders show on this phone (the pure part of
 * `NoteReminders`; the desktop's `note_reminders.rs` follows the same
 * rules). A reminder shows once per time: what was shown (note id -> its
 * time) is kept, so a restart or a second alarm doesn't show it again, and
 * a new time (snooze) is a new reminder. One that came due while the phone
 * was off is shown once if it's less than [LATE_MS] late, else skipped.
 */
object ReminderPlan {
    /** A reminder this late is skipped, not shown. */
    const val LATE_MS = 24L * 3600 * 1000
    private const val MINUTE = 60_000L

    /** One note's reminder, as kept for the alarm (the app may not be
     *  running when it goes off). */
    data class Entry(val id: String, val at: Long?, val done: Boolean, val title: String, val line: String)

    sealed interface Due {
        /** No reminder, Done, or already shown. */
        data object Nothing : Due
        /** Not yet: at this time. */
        data class Later(val at: Long) : Due
        /** Show it now. */
        data object Show : Due
        /** Too late: count it as shown, say nothing. */
        data object Skip : Due
    }

    /** One reminder at [now] ([shown]: the time it was last shown for). */
    fun due(at: Long?, done: Boolean, shown: Long?, now: Long): Due = when {
        at == null || done || shown == at -> Due.Nothing
        at > now -> Due.Later(at)
        now - at < LATE_MS -> Due.Show
        else -> Due.Skip
    }

    /**
     * A look at every note at [now]: [show] these, take down [dismiss]
     * (shown before, now Done, given a new time, or gone), set the alarm for
     * [next], and keep [shown].
     */
    data class Plan(val show: List<String>, val dismiss: List<String>, val next: Long?, val shown: Map<String, Long>)

    fun plan(entries: Collection<Entry>, shown: Map<String, Long>, now: Long): Plan {
        val show = mutableListOf<String>()
        val kept = HashMap<String, Long>()
        var next: Long? = null
        for (e in entries) {
            val before = shown[e.id]
            when (val d = due(e.at, e.done, before, now)) {
                Due.Nothing -> if (e.at != null && !e.done && before == e.at) kept[e.id] = e.at
                is Due.Later -> next = next?.let { minOf(it, d.at) } ?: d.at
                Due.Show -> {
                    show.add(e.id)
                    kept[e.id] = e.at!!
                }
                Due.Skip -> kept[e.id] = e.at!!
            }
        }
        val dismiss = shown.keys.filter { it !in kept }
        return Plan(show, dismiss, next, kept)
    }

    enum class Snooze { TEN_MIN, HOUR, TOMORROW }

    /** When a snooze ends. */
    fun snoozeUntil(s: Snooze, now: Long, tz: TimeZone = TimeZone.getDefault()): Long = when (s) {
        Snooze.TEN_MIN -> now + 10 * MINUTE
        Snooze.HOUR -> now + 60 * MINUTE
        Snooze.TOMORROW -> tomorrowMorning(now, tz)
    }

    private fun cal(now: Long, tz: TimeZone) = Calendar.getInstance(tz).apply { timeInMillis = now }

    private fun Calendar.at(hour: Int): Calendar = apply {
        set(Calendar.HOUR_OF_DAY, hour)
        set(Calendar.MINUTE, 0)
        set(Calendar.SECOND, 0)
        set(Calendar.MILLISECOND, 0)
    }

    /** "Later today": 18:00 before 17:00, else three hours on (on the
     *  hour) while that's still today; null late in the evening. */
    fun laterToday(now: Long, tz: TimeZone = TimeZone.getDefault()): Long? {
        val c = cal(now, tz)
        if (c.get(Calendar.HOUR_OF_DAY) < 17) return c.at(18).timeInMillis
        val day = c.get(Calendar.DAY_OF_YEAR)
        c.set(Calendar.MINUTE, 0)
        c.set(Calendar.SECOND, 0)
        c.set(Calendar.MILLISECOND, 0)
        c.add(Calendar.HOUR_OF_DAY, 3)
        return if (c.get(Calendar.DAY_OF_YEAR) == day) c.timeInMillis else null
    }

    /** Tomorrow at 9:00. */
    fun tomorrowMorning(now: Long, tz: TimeZone = TimeZone.getDefault()): Long =
        cal(now, tz).apply { add(Calendar.DAY_OF_YEAR, 1) }.at(9).timeInMillis

    /** Next Monday at 9:00 (a week on, from a Monday). */
    fun nextWeek(now: Long, tz: TimeZone = TimeZone.getDefault()): Long {
        val c = cal(now, tz)
        // Calendar: Sunday 1 ... Saturday 7; days to the next Monday, 1..7.
        val days = ((Calendar.MONDAY - c.get(Calendar.DAY_OF_WEEK)) + 7) % 7
        c.add(Calendar.DAY_OF_YEAR, if (days == 0) 7 else days)
        return c.at(9).timeInMillis
    }

    /** The body's first line that isn't blank (checklist boxes left out),
     *  for the notification. */
    fun firstLine(body: String): String {
        val line = body.lineSequence().map { it.trim() }.firstOrNull { it.isNotEmpty() } ?: ""
        return line.replace(Regex("^\\[( |x|X)] "), "").take(200)
    }
}
