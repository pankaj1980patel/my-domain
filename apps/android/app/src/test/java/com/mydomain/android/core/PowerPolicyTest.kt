package com.mydomain.android.core

import com.mydomain.android.core.PowerPolicy.Conditions
import com.mydomain.android.core.PowerPolicy.Mode
import com.mydomain.android.core.PowerPolicy.Paused
import com.mydomain.android.core.PowerPolicy.Reason
import com.mydomain.android.core.PowerPolicy.Settings
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class PowerPolicyTest {
    private val auto = Settings()

    @Test
    fun nothingWaitsOnAFullBatteryOnWifi() {
        val p = PowerPolicy.decide(auto, Conditions(level = 80))
        assertFalse(p.saving)
        assertTrue(p.heavyAllowed)
        assertTrue(p.paused.isEmpty())
        assertEquals(0L, p.meteredCap)
        assertTrue(p.backupNetworkOk)
        assertEquals(PowerPolicy.Plan.NONE.copy(), p.copy(backupNetworkOk = true))
    }

    @Test
    fun batterySaverPausesHeavyWork() {
        val p = PowerPolicy.decide(auto, Conditions(powerSave = true, level = 90, charging = true))
        assertTrue(p.saving)
        assertFalse(p.heavyAllowed)
        assertEquals(listOf(Reason.BATTERY_SAVER), p.reasons)
        assertEquals(setOf(Paused.FOLDER_SYNC, Paused.PHOTO_BACKUP, Paused.CLIP_PICTURES, Paused.AUTO_RESTARTS), p.paused)
        // Not metered: no cap.
        assertEquals(0L, p.meteredCap)
    }

    @Test
    fun lowBatteryCountsOnlyWhenNotCharging() {
        assertEquals(listOf(Reason.LOW_BATTERY), PowerPolicy.decide(auto, Conditions(level = 15)).reasons)
        assertFalse(PowerPolicy.decide(auto, Conditions(level = 16)).saving)
        assertFalse(PowerPolicy.decide(auto, Conditions(level = 5, charging = true)).saving)
        // Unknown level: not low.
        assertFalse(PowerPolicy.decide(auto, Conditions(level = null)).saving)
        // Own threshold.
        assertTrue(PowerPolicy.decide(auto.copy(threshold = 30), Conditions(level = 25)).saving)
    }

    @Test
    fun thresholdIsKeptInRange() {
        assertEquals(PowerPolicy.MIN_THRESHOLD, PowerPolicy.clampThreshold(0))
        assertEquals(PowerPolicy.MAX_THRESHOLD, PowerPolicy.clampThreshold(99))
        assertEquals(20, PowerPolicy.clampThreshold(20))
        // A stored 0 doesn't turn "low battery" off.
        assertTrue(PowerPolicy.decide(auto.copy(threshold = 0), Conditions(level = 4)).saving)
    }

    @Test
    fun dozingSaves() {
        val p = PowerPolicy.decide(auto, Conditions(idle = true, level = 70))
        assertEquals(listOf(Reason.IDLE), p.reasons)
        assertTrue(p.pauses(Paused.FOLDER_SYNC))
    }

    @Test
    fun severalReasonsAreAllShown() {
        val p = PowerPolicy.decide(auto, Conditions(powerSave = true, level = 3, idle = true, metered = true))
        assertEquals(listOf(Reason.BATTERY_SAVER, Reason.LOW_BATTERY, Reason.IDLE, Reason.METERED), p.reasons)
    }

    @Test
    fun alwaysAndOff() {
        val always = PowerPolicy.decide(auto.copy(mode = Mode.ALWAYS), Conditions(level = 100, charging = true))
        assertTrue(always.saving)
        assertEquals(listOf(Reason.ALWAYS), always.reasons)
        val off = PowerPolicy.decide(auto.copy(mode = Mode.OFF), Conditions(powerSave = true, level = 2, idle = true))
        assertFalse(off.saving)
        assertTrue(off.paused.isEmpty())
    }

    @Test
    fun meteredHoldsBigFilesAndBackupUnlessMobileDataIsAllowed() {
        val p = PowerPolicy.decide(auto, Conditions(level = 90, metered = true))
        assertFalse(p.saving)
        assertTrue(p.heavyAllowed)
        assertEquals(listOf(Reason.METERED), p.reasons)
        assertEquals(setOf(Paused.BIG_SYNC_FILES, Paused.PHOTO_BACKUP), p.paused)
        assertEquals(10L * 1024 * 1024, p.meteredCap)
        assertFalse(p.backupNetworkOk)

        val allowed = PowerPolicy.decide(auto.copy(mobileData = true), Conditions(level = 90, metered = true))
        assertTrue(allowed.paused.isEmpty())
        assertEquals(0L, allowed.meteredCap)
        assertTrue(allowed.backupNetworkOk)

        // Mode Off is about the battery: the data rule still applies.
        val off = PowerPolicy.decide(auto.copy(mode = Mode.OFF), Conditions(metered = true))
        assertEquals(PowerPolicy.METERED_CAP, off.meteredCap)
    }

    @Test
    fun modeKeysRoundTrip() {
        Mode.entries.forEach { assertEquals(it, Mode.of(it.key)) }
        assertEquals(Mode.AUTO, Mode.of(null))
        assertEquals(Mode.AUTO, Mode.of("nonsense"))
    }
}
