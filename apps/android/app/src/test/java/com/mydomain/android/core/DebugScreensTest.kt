package com.mydomain.android.core

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class DebugScreensTest {
    @Test fun everyScreenOpensByItsKey() {
        for (s in DebugScreen.entries) {
            val r = DebugScreens.parse(s.key, device = SampleData.MACBOOK)
            assertEquals(s, r?.screen)
        }
    }

    @Test fun namesIgnoreCaseDashesAndUnderscores() {
        assertEquals(DebugScreen.READ_LATER, DebugScreens.parse("read-later")?.screen)
        assertEquals(DebugScreen.READ_LATER, DebugScreens.parse("READ_LATER")?.screen)
        assertEquals(DebugScreen.LINKS_MAP, DebugScreens.parse("linksmap")?.screen)
        assertEquals(DebugScreen.SETUP_PHONE, DebugScreens.parse("setup")?.screen)
        assertEquals(DebugScreen.SIGN_IN, DebugScreens.parse("sign-in")?.screen)
    }

    @Test fun unknownOrMissingNamesOpenNothing() {
        assertNull(DebugScreens.parse(null))
        assertNull(DebugScreens.parse(""))
        assertNull(DebugScreens.parse("nowhere"))
    }

    @Test fun aDevicePageNeedsTheDevice() {
        assertNull(DebugScreens.parse("device"))
        assertNull(DebugScreens.parse("device", device = "  "))
        assertEquals(ScreenRequest(DebugScreen.DEVICE, device = "n1"), DebugScreens.parse("device", device = " n1 "))
    }

    @Test fun onlyTheScreenThatUsesAnExtraKeepsIt() {
        assertEquals(ScreenRequest(DebugScreen.HOME), DebugScreens.parse("home", device = "n1", topic = "notes", query = "x"))
        assertEquals(ScreenRequest(DebugScreen.HELP, topic = "notes"), DebugScreens.parse("help", topic = "notes"))
        assertEquals(ScreenRequest(DebugScreen.SEARCH, query = "boarding"), DebugScreens.parse("search", query = "boarding"))
        assertEquals(ScreenRequest(DebugScreen.SEARCH), DebugScreens.parse("search", query = " "))
    }

    @Test fun sampleDevicesAreClearlyMadeUp() {
        val nodes = SampleData.devices.map { it.node } + SampleData.ME
        assertTrue(nodes.all { it.startsWith(SampleData.PREFIX) })
        assertEquals(nodes.size, nodes.toSet().size)
        val now = 1_700_000_000_000
        // Everything else in the sample talks about those devices only.
        assertTrue(nodes.containsAll(SampleData.statuses(now).keys))
        assertTrue(nodes.containsAll(SampleData.playing(now).keys))
        assertTrue(SampleData.messages(now).all { it.node in nodes })
        assertTrue(SampleData.transfers().all { it.node in nodes })
        assertTrue(SampleData.transferRecords(now).all { it.node in nodes })
        assertTrue(SampleData.activityLog(now).all { it.node in nodes })
        assertTrue(SampleData.clips(now).all { it.source == null || it.source in nodes })
    }

    @Test fun connectedSampleDevicesHaveAQualityAndTheRestAStage() {
        for (d in SampleData.devices) {
            if (d.link != null) assertTrue(d.quality != null && d.stage == null)
            else assertTrue(d.quality == null && d.stage != null)
        }
    }
}
