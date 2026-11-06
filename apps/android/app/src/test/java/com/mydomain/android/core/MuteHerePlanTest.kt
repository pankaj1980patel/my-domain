package com.mydomain.android.core

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/** The same cases as the desktop's `mute_here.rs` tests. */
class MuteHerePlanTest {
    @Test fun onlyWhatWeMutedIsUndone() {
        // Not muted: mute, and undo it after.
        val (set, saved) = MuteHerePlan.begin(false)
        assertEquals(true, set)
        assertEquals(false, MuteHerePlan.end(saved, true))
        // Unmuted by hand meanwhile: leave it.
        assertNull(MuteHerePlan.end(saved, false))
        // Muted already: nothing to set, nothing to undo.
        val (set2, saved2) = MuteHerePlan.begin(true)
        assertNull(set2)
        assertNull(MuteHerePlan.end(saved2, true))
        // Nothing written down (never muted, or undone already).
        assertNull(MuteHerePlan.end(null, true))
    }

    @Test fun afterACrashTheNextStartUndoesIt() {
        // What's left in the preferences after the app died mid-stream.
        assertEquals(false, MuteHerePlan.end(MuteHerePlan.MUTED_BY_US, true))
        assertNull(MuteHerePlan.end(MuteHerePlan.WAS_MUTED, true))
        assertNull(MuteHerePlan.end("garbage", true))
    }

    @Test fun mutedWhileAComputerWithTheSwitchPlays() {
        val on = { n: String -> n == "pc-a" }
        assertTrue(MuteHerePlan.wanted(setOf("pc-b", "pc-a"), on))
        assertFalse(MuteHerePlan.wanted(setOf("pc-b"), on))
        assertFalse("nothing playing: restore", MuteHerePlan.wanted(emptySet(), on))
    }

    @Test fun routesAndTheBadge() {
        assertEquals("auto", AudioRoutes.of(null))
        assertEquals("auto", AudioRoutes.of("anything"))
        AudioRoutes.ALL.forEach { assertEquals(it, AudioRoutes.of(it)) }
        assertTrue(AudioRoutes.viaBluetooth("bluetooth"))
        assertFalse(AudioRoutes.viaBluetooth("network"))
        assertFalse(AudioRoutes.viaBluetooth(""))
    }
}
