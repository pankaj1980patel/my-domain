package com.mydomain.android.core

import com.mydomain.android.core.ReminderPlan.Due
import com.mydomain.android.core.ReminderPlan.Entry
import com.mydomain.android.core.ReminderPlan.Snooze
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import java.util.Calendar
import java.util.TimeZone

class ReminderPlanTest {
    private val h = 3600_000L
    private val tz: TimeZone = TimeZone.getTimeZone("Europe/London")

    /** [y]-[m]-[d] [hh]:[mm] in [tz], as unix ms. */
    private fun at(y: Int, m: Int, d: Int, hh: Int, mm: Int = 0): Long =
        Calendar.getInstance(tz).apply {
            clear()
            set(y, m - 1, d, hh, mm, 0)
        }.timeInMillis

    private fun e(id: String, at: Long?, done: Boolean = false) = Entry(id, at, done, id, "")

    @Test
    fun dueByTheRules() {
        val now = 100 * h
        assertEquals(Due.Nothing, ReminderPlan.due(null, false, null, now))
        assertEquals(Due.Later(now + 5), ReminderPlan.due(now + 5, false, null, now))
        assertEquals(Due.Show, ReminderPlan.due(now, false, null, now))
        assertEquals(Due.Show, ReminderPlan.due(now - 23 * h, false, null, now))
        assertEquals(Due.Skip, ReminderPlan.due(now - 24 * h, false, null, now))
        assertEquals(Due.Nothing, ReminderPlan.due(now - h, true, null, now))
        assertEquals(Due.Nothing, ReminderPlan.due(now - h, false, now - h, now))
        // Shown for an older time (snoozed since): a new reminder.
        assertEquals(Due.Show, ReminderPlan.due(now - h, false, now - 2 * h, now))
    }

    @Test
    fun theAlarmIsForTheSoonestPending() {
        val now = 100 * h
        val p = ReminderPlan.plan(
            listOf(e("a", now + 3 * h), e("b", now + h), e("c", now + 30, done = true), e("d", null)),
            emptyMap(), now,
        )
        assertTrue(p.show.isEmpty())
        assertEquals(now + h, p.next)
        assertTrue(p.dismiss.isEmpty())
    }

    @Test
    fun afterARestartAMissedOneShowsOnceIfUnderADayLate() {
        val now = 100 * h
        val notes = listOf(e("recent", now - h), e("old", now - 48 * h))
        val p = ReminderPlan.plan(notes, emptyMap(), now)
        assertEquals(listOf("recent"), p.show)
        assertEquals(now - 48 * h, p.shown["old"])
        val again = ReminderPlan.plan(notes, p.shown, now + 60_000)
        assertTrue(again.show.isEmpty())
        assertEquals(p.shown, again.shown)
        assertNull(again.next)
    }

    @Test
    fun doneOrSnoozedElsewhereTakesTheNotificationDown() {
        val now = 100 * h
        val shown = mapOf("s" to now - h, "d" to now - h, "gone" to now - h, "still" to now - h)
        val p = ReminderPlan.plan(
            listOf(e("s", now + 10 * 60_000), e("d", now - h, done = true), e("still", now - h)),
            shown, now,
        )
        assertEquals(setOf("s", "d", "gone"), p.dismiss.toSet())
        assertEquals(mapOf("still" to now - h), p.shown)
        assertEquals(now + 10 * 60_000, p.next)
    }

    @Test
    fun snoozeTimes() {
        val now = at(2026, 11, 5, 14, 37)
        assertEquals(now + 10 * 60_000, ReminderPlan.snoozeUntil(Snooze.TEN_MIN, now, tz))
        assertEquals(now + h, ReminderPlan.snoozeUntil(Snooze.HOUR, now, tz))
        assertEquals(at(2026, 11, 6, 9), ReminderPlan.snoozeUntil(Snooze.TOMORROW, now, tz))
        // Across a month end.
        assertEquals(at(2026, 12, 1, 9), ReminderPlan.tomorrowMorning(at(2026, 11, 30, 23, 50), tz))
    }

    @Test
    fun quickPicks() {
        // Thursday 5 Nov 2026.
        assertEquals(at(2026, 11, 5, 18), ReminderPlan.laterToday(at(2026, 11, 5, 9, 15), tz))
        assertEquals(at(2026, 11, 5, 20), ReminderPlan.laterToday(at(2026, 11, 5, 17, 40), tz))
        assertNull(ReminderPlan.laterToday(at(2026, 11, 5, 21, 5), tz))
        assertEquals(at(2026, 11, 9, 9), ReminderPlan.nextWeek(at(2026, 11, 5, 10), tz))
        // From a Monday: a week on; from a Sunday: the next day.
        assertEquals(at(2026, 11, 16, 9), ReminderPlan.nextWeek(at(2026, 11, 9, 8), tz))
        assertEquals(at(2026, 11, 9, 9), ReminderPlan.nextWeek(at(2026, 11, 8, 22), tz))
    }

    @Test
    fun theNotificationLine() {
        assertEquals("milk", ReminderPlan.firstLine("\n  \n[ ] milk\n[x] eggs"))
        assertEquals("", ReminderPlan.firstLine(""))
        assertEquals(200, ReminderPlan.firstLine("x".repeat(500)).length)
    }
}
