package com.mydomain.android.core

import org.junit.Assert.assertEquals
import org.junit.Assert.fail
import org.junit.Test

class DeviceIdPlanTest {
    private val noRandom: () -> String = { fail("no random id expected"); "" }

    @Test
    fun aKeptIdAlwaysWins() {
        // Even when the phone now reports something else.
        assertEquals("kept-1", DeviceIdPlan.choose("kept-1", "356938035643809", "a1b2c3d4e5f60718", noRandom))
        assertEquals("kept-1", DeviceIdPlan.choose(" kept-1\n", null, "a1b2c3d4e5f60718", noRandom))
    }

    @Test
    fun anOldInstallThatUsedTheImeiKeepsIt() {
        // Android 9 or older, READ_PHONE_STATE granted by an earlier version.
        assertEquals("356938035643809", DeviceIdPlan.choose(null, "356938035643809", "a1b2c3d4e5f60718", noRandom))
    }

    @Test
    fun otherwiseAndroidIdAsEarlierVersionsDid() {
        // What earlier versions used on Android 10+ and where the IMEI wasn't
        // granted, and what a new install uses: the same value.
        assertEquals("a1b2c3d4e5f60718", DeviceIdPlan.choose(null, null, "a1b2c3d4e5f60718", noRandom))
        assertEquals("a1b2c3d4e5f60718", DeviceIdPlan.choose("", "  ", "a1b2c3d4e5f60718", noRandom))
    }

    @Test
    fun noUsableAndroidIdGivesARandomOne() {
        assertEquals("rnd", DeviceIdPlan.choose(null, null, null) { "rnd" })
        assertEquals("rnd", DeviceIdPlan.choose(null, null, "") { "rnd" })
    }
}
