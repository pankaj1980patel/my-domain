package com.mydomain.android.core

import com.mydomain.android.core.TimerPlan.Entry
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class TimerPlanTest {
    private val min = 60_000L

    private fun e(id: String, state: String, endsAt: Long, ringOn: List<String>, left: Long = 0) =
        Entry(id, id, state, endsAt, left, 5 * min, ringOn)

    @Test
    fun ringsHereOnlyWhenThisPhoneWasPicked() {
        val now = 100 * min
        val list = listOf(
            e("mine", "running", now - 1, listOf("me", "mac")),
            e("theirs", "running", now - 1, listOf("mac")),
            e("paused", "paused", 0, listOf("me"), left = min),
            e("later", "running", now + min, listOf("me")),
            e("laterTheirs", "running", now + 10, listOf("mac")),
        )
        val p = TimerPlan.plan(list, "me", emptyMap(), now)
        assertEquals(listOf("mine"), p.ring)
        assertEquals(now + min, p.next)
        assertEquals(mapOf("mine" to now - 1), p.rung)
        assertTrue(p.quiet.isEmpty())
    }

    @Test
    fun ringsOncePerEndAndAgainAfterMoreTime() {
        val now = 100 * min
        val first = TimerPlan.plan(listOf(e("t", "running", now, listOf("me"))), "me", emptyMap(), now)
        assertEquals(listOf("t"), first.ring)
        val again = TimerPlan.plan(listOf(e("t", "running", now, listOf("me"))), "me", first.rung, now + 1000)
        assertTrue(again.ring.isEmpty())
        // "+1 min": quiet now, rings again at the new end.
        val more = listOf(e("t", "running", now + min, listOf("me")))
        val p = TimerPlan.plan(more, "me", again.rung, now + 2000)
        assertTrue(p.ring.isEmpty())
        assertEquals(listOf("t"), p.quiet)
        assertEquals(now + min, p.next)
        assertEquals(listOf("t"), TimerPlan.plan(more, "me", p.rung, now + min).ring)
    }

    @Test
    fun stoppedOrGoneGoesQuiet() {
        val now = 100 * min
        val rung = mapOf("t" to now - 5)
        val p = TimerPlan.plan(emptyList(), "me", rung, now)
        assertEquals(listOf("t"), p.quiet)
        assertNull(p.next)
        assertTrue(p.rung.isEmpty())
    }

    @Test
    fun tooLateDoesNotRing() {
        val now = 100 * min
        val p = TimerPlan.plan(listOf(e("old", "running", now - TimerPlan.LATE_MS, listOf("me"))), "me", emptyMap(), now)
        assertTrue(p.ring.isEmpty())
        assertEquals(mapOf("old" to now - TimerPlan.LATE_MS), p.rung)
    }

    @Test
    fun theClock() {
        assertEquals("0:00", TimerPlan.clock(0))
        assertEquals("0:07", TimerPlan.clock(7_000))
        assertEquals("0:07", TimerPlan.clock(6_001))
        assertEquals("4:32", TimerPlan.clock(4 * min + 32_000))
        assertEquals("1:05:03", TimerPlan.clock(3_903_000))
        assertEquals("0:00", TimerPlan.clock(-5))
    }

    @Test
    fun whatIsTypedIn() {
        assertEquals(5 * min, TimerPlan.durationOf("", "5", ""))
        assertEquals(3_903_000L, TimerPlan.durationOf("1", "5", "3"))
        assertEquals(90_000L, TimerPlan.durationOf("0", "0", "90"))
        assertNull(TimerPlan.durationOf("", "", ""))
        assertNull(TimerPlan.durationOf("", "", "0.5"))
        assertNull(TimerPlan.durationOf("x", "1", ""))
        assertNull(TimerPlan.durationOf("-1", "5", ""))
        assertNull(TimerPlan.durationOf("169", "", ""))
        assertEquals(TimerPlan.MAX_MS, TimerPlan.durationOf("168", "", ""))
    }

    @Test
    fun leftAndProgress() {
        val now = 100 * min
        assertEquals(2 * min, TimerPlan.remaining("running", now + 2 * min, 0, now))
        assertEquals(0L, TimerPlan.remaining("running", now - 1, 0, now))
        assertEquals(min, TimerPlan.remaining("paused", 0, min, now))
        assertEquals(0.6f, TimerPlan.progress("running", now + 2 * min, 0, 5 * min, now), 0.001f)
        assertEquals(1f, TimerPlan.progress("running", now - 1, 0, 5 * min, now), 0.001f)
        assertTrue(TimerPlan.ended("running", now, now))
        assertTrue(!TimerPlan.ended("paused", 0, now))
    }

    @Test
    fun notificationButtonsChangeTheKeptTimerAtOnce() {
        val now = 100 * min
        val run = e("t", "running", now + 2 * min, listOf("me"))
        assertNull(TimerPlan.applied(run, "dismiss", now))
        assertNull(TimerPlan.applied(run, "cancel", now))
        val paused = TimerPlan.applied(run, "pause", now)!!
        assertEquals(Triple("paused", 0L, 2 * min), Triple(paused.state, paused.endsAt, paused.remainingMs))
        val resumed = TimerPlan.applied(paused, "resume", now + 10 * min)!!
        assertEquals(now + 12 * min, resumed.endsAt)
        assertEquals(now + 3 * min, TimerPlan.applied(run, "add", now)!!.endsAt)
        assertEquals(3 * min, TimerPlan.applied(paused, "add", now)!!.remainingMs)
        // Ended: a minute from now, a minute long.
        val again = TimerPlan.applied(run, "add", now + 5 * min)!!
        assertEquals(now + 6 * min to min, again.endsAt to again.durationMs)
        // Pausing an ended one does nothing.
        assertEquals(run, TimerPlan.applied(run, "pause", now + 5 * min))
    }

    @Test
    fun ongoingIsRunningThenPaused() {
        val now = 100 * min
        val list = listOf(
            e("p", "paused", 0, emptyList(), left = min),
            e("late", "running", now + 5 * min, emptyList()),
            e("soon", "running", now + min, emptyList()),
            e("ended", "running", now - 1, emptyList()),
        )
        assertEquals(listOf("soon", "late", "p"), TimerPlan.ongoing(list, now).map { it.id })
    }
}
