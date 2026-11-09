package com.mydomain.android.core

import com.mydomain.android.core.Dest.Kind
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class NavStackTest {
    @Test
    fun backGoesWhereYouCameFrom() {
        // Settings -> Set up this phone -> Help: Back, Back, Back.
        var s = emptyList<Dest>()
        s = NavStack.push(s, Dest(Kind.Settings))
        s = NavStack.push(s, Dest(Kind.Setup))
        s = NavStack.push(s, Dest(Kind.Help, "setup-phone"))
        assertEquals(Kind.Help, s.last().kind)
        s = NavStack.pop(s)
        assertEquals(Kind.Setup, s.last().kind)
        s = NavStack.pop(s)
        assertEquals(Kind.Settings, s.last().kind)
        s = NavStack.pop(s)
        assertTrue(s.isEmpty())
        // Popping nothing is nothing.
        assertTrue(NavStack.pop(s).isEmpty())
    }

    @Test
    fun aDeviceFromSearchComesBackToSearch() {
        var s = listOf(Dest(Kind.Search))
        s = NavStack.push(s, Dest(Kind.Device, "node-1", "msg:42"))
        s = NavStack.push(s, Dest(Kind.Health))
        assertEquals(listOf(Kind.Search, Kind.Device, Kind.Health), s.map { it.kind })
        assertEquals(Dest(Kind.Device, "node-1", "msg:42"), NavStack.pop(s).last())
    }

    @Test
    fun theSameScreenTwiceIsOneEntry() {
        val s = NavStack.push(listOf(Dest(Kind.Notes)), Dest(Kind.Notes))
        assertEquals(1, s.size)
        // Another device is another screen.
        assertEquals(2, NavStack.push(listOf(Dest(Kind.Device, "a")), Dest(Kind.Device, "b")).size)
    }

    @Test
    fun aTabChangeReplacesTheTop() {
        val s = NavStack.replaceTop(listOf(Dest(Kind.Settings), Dest(Kind.Files, "files")), Dest(Kind.Files, "sync"))
        assertEquals(listOf(Dest(Kind.Settings), Dest(Kind.Files, "sync")), s)
        assertEquals(listOf(Dest(Kind.Timers)), NavStack.replaceTop(emptyList(), Dest(Kind.Timers)))
    }

    @Test
    fun withoutLeavesOutOneKindKeepingOrder() {
        val s = listOf(Dest(Kind.Settings), Dest(Kind.Onboarding), Dest(Kind.Health), Dest(Kind.Help))
        assertEquals(listOf(Kind.Settings, Kind.Help), NavStack.without(s, Kind.Onboarding, Kind.Health).map { it.kind })
    }

    @Test
    fun keysIgnoreArgumentsExceptTheDevice() {
        assertEquals(NavStack.key(Dest(Kind.Help)), NavStack.key(Dest(Kind.Help, "notes")))
        assertEquals(NavStack.key(Dest(Kind.Files, "files")), NavStack.key(Dest(Kind.Files, "sync", "t1")))
        assertTrue(NavStack.key(Dest(Kind.Device, "a")) != NavStack.key(Dest(Kind.Device, "b")))
    }

    @Test
    fun savedTextRoundTrips() {
        val s = listOf(
            Dest(Kind.Settings, "group:Health"),
            Dest(Kind.Device, "a|b\\c", "msg:1|2"),
            Dest(Kind.Help, null, ""),
            Dest(Kind.Notes, "\\0", "\\p"),
            Dest(Kind.ConnHealth),
        )
        assertEquals(s, NavStack.decode(NavStack.encode(s)))
    }

    @Test
    fun unreadableLinesAreLeftOut() {
        val lines = listOf("Timers|\\0|\\0", "NoSuchScreen|\\0|\\0", "garbage", "Help|notes|\\0")
        assertEquals(listOf(Dest(Kind.Timers), Dest(Kind.Help, "notes")), NavStack.decode(lines))
    }
}
