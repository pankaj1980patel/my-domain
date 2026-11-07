package com.mydomain.android.core

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class PermsPlanTest {
    @Test fun aPhoneAsksOnlyAboutItsScreen() {
        assertTrue(PermsPlan.asksHere("screen"))
        assertFalse(PermsPlan.asksHere("control"))
        assertFalse(PermsPlan.asksHere("kvm"))
        assertFalse(PermsPlan.asksHere("everything"))
        assertTrue(PermsPlan.known("kvm"))
        assertFalse(PermsPlan.known("input"))
    }

    @Test fun allowingTheScreenStillGoesThroughAndroidsPrompt() {
        assertTrue(PermsPlan.startsCapture("screen", PermsPlan.ONCE))
        assertTrue(PermsPlan.startsCapture("screen", PermsPlan.ALWAYS))
        assertFalse(PermsPlan.startsCapture("screen", PermsPlan.DENY))
        assertFalse(PermsPlan.startsCapture("control", PermsPlan.ALWAYS))
    }

    @Test fun sharingFromTheComputersPageIsAnAllowOnce() {
        assertEquals(PermsPlan.ONCE, PermsPlan.shareAnswer(false))
        assertNull(PermsPlan.shareAnswer(true))
    }

    @Test fun aRefusalIsSaidOnceInAWhilePerComputerAndPermission() {
        val t = PermsPlan.Throttle(1_000)
        assertTrue(t.due("mac", "control", 0))
        assertFalse(t.due("mac", "control", 500))
        assertTrue(t.due("mac", "screen", 500))
        assertTrue(t.due("pc", "control", 500))
        assertTrue(t.due("mac", "control", 1_000))
        // A clock that went back doesn't silence it for good.
        assertTrue(t.due("mac", "control", 10))
    }
}
