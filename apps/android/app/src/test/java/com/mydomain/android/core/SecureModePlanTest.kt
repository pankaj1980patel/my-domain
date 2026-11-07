package com.mydomain.android.core

import com.mydomain.android.core.SecureModePlan.Path
import com.mydomain.android.core.SecureModePlan.State
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class SecureModePlanTest {
    private val me = "com.mydomain.android/com.mydomain.android.PhoneHelper"
    private val other = "com.google.android.marvin.talkback/com.google.android.marvin.talkback.TalkBackService"

    @Test
    fun addsToEmptyAndNull() {
        assertEquals(me, SecureModePlan.add(null, me))
        assertEquals(me, SecureModePlan.add("", me))
    }

    @Test
    fun addKeepsOthersAndDoesNotRepeat() {
        assertEquals("$other:$me", SecureModePlan.add(other, me))
        assertEquals("$other:$me", SecureModePlan.add("$other:$me", me))
        // Already there in its short form or another case: unchanged.
        val short = "com.mydomain.android/.PhoneHelper"
        assertEquals("$short:$other", SecureModePlan.add("$short:$other", me))
        assertEquals(other, SecureModePlan.add(other, other.uppercase()))
    }

    @Test
    fun removeKeepsOthers() {
        assertEquals(other, SecureModePlan.remove("$me:$other", me))
        assertEquals(other, SecureModePlan.remove("$other:$me", me))
        assertEquals("", SecureModePlan.remove(me, me))
        assertEquals("", SecureModePlan.remove(null, me))
        assertEquals(other, SecureModePlan.remove("$other:com.mydomain.android/.PhoneHelper", me))
        // Stray separators and spaces go.
        assertEquals("a/b:c/d", SecureModePlan.remove(":a/b::$me: c/d :", me))
    }

    @Test
    fun containsMatchesExactly() {
        assertTrue(SecureModePlan.contains("$other:$me", me))
        assertTrue(SecureModePlan.contains(me.uppercase(), me))
        assertFalse(SecureModePlan.contains(other, me))
        assertFalse(SecureModePlan.contains("${me}2", me))
        assertFalse(SecureModePlan.contains(null, me))
    }

    @Test
    fun states() {
        assertEquals(State.HELPER_ON, SecureModePlan.state(wanted = false, enabled = true))
        assertEquals(State.HELPER_OFF, SecureModePlan.state(wanted = false, enabled = false))
        assertEquals(State.SECURE, SecureModePlan.state(wanted = true, enabled = false))
        assertEquals(State.TURNING_OFF, SecureModePlan.state(wanted = true, enabled = true))
        assertTrue(SecureModePlan.shownOn(State.SECURE))
        assertTrue(SecureModePlan.shownOn(State.TURNING_OFF))
        assertFalse(SecureModePlan.shownOn(State.HELPER_ON))
        assertFalse(SecureModePlan.shownOn(State.HELPER_OFF))
    }

    @Test
    fun enablePaths() {
        assertEquals(Path.NONE, SecureModePlan.enablePath(enabled = true, canWriteSecure = false))
        assertEquals(Path.WRITE, SecureModePlan.enablePath(enabled = false, canWriteSecure = true))
        assertEquals(Path.SETTINGS, SecureModePlan.enablePath(enabled = false, canWriteSecure = false))
    }

    @Test
    fun idleChoices() {
        assertEquals(15, SecureModePlan.idleMinutes(15))
        assertEquals(0, SecureModePlan.idleMinutes(7))
        assertEquals(0, SecureModePlan.idleMinutes(-1))
    }

    @Test
    fun idleTimer() {
        val m = SecureModePlan.MINUTE_MS
        assertFalse(SecureModePlan.idleDue(0, 0, 999 * m, controlling = false))
        assertFalse(SecureModePlan.idleDue(5, 0, 4 * m, controlling = false))
        assertTrue(SecureModePlan.idleDue(5, 0, 5 * m, controlling = false))
        // Never while a computer may control the screen.
        assertFalse(SecureModePlan.idleDue(5, 0, 60 * m, controlling = true))

        assertNull(SecureModePlan.nextCheckMs(0, 0, 0, controlling = false))
        assertEquals(3 * m, SecureModePlan.nextCheckMs(5, 0, 2 * m, controlling = false))
        assertEquals(m, SecureModePlan.nextCheckMs(5, 0, 2 * m, controlling = true))
        // Overdue: soon, not negative.
        assertEquals(1_000L, SecureModePlan.nextCheckMs(5, 0, 9 * m, controlling = false))
    }

    @Test
    fun afterSession() {
        assertTrue(SecureModePlan.afterSessionDue(afterSession = true, controlWasAllowed = true))
        assertFalse(SecureModePlan.afterSessionDue(afterSession = true, controlWasAllowed = false))
        assertFalse(SecureModePlan.afterSessionDue(afterSession = false, controlWasAllowed = true))
    }

    @Test
    fun tellsOncePerBurst() {
        assertTrue(SecureModePlan.shouldTell(0, SecureModePlan.TELL_EVERY_MS))
        assertFalse(SecureModePlan.shouldTell(1_000, 2_000))
    }
}
