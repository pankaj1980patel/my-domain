package com.mydomain.android.core

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class KeyVerifyPlanTest {
    @Test
    fun onlyVerifyLinksAreTakenAsCodes() {
        assertTrue(KeyVerifyPlan.isVerifyLink("mydomain://verify?v=1&f=a"))
        assertTrue(KeyVerifyPlan.isVerifyLink("  MYDOMAIN://VERIFY?v=1"))
        assertFalse(KeyVerifyPlan.isVerifyLink("mydomain://join?v=1"))
        assertFalse(KeyVerifyPlan.isVerifyLink("https://example.com/verify?"))
        assertFalse(KeyVerifyPlan.isVerifyLink(null))
    }

    @Test
    fun aScanIsVerifiedOnlyWithANodeAndNoProblem() {
        assertEquals(KeyVerifyPlan.Outcome.Verified, KeyVerifyPlan.outcome("desk", null))
        assertEquals(KeyVerifyPlan.Outcome.NotACode, KeyVerifyPlan.outcome(null, null))
        assertEquals(KeyVerifyPlan.Outcome.NotForThisDevice, KeyVerifyPlan.outcome(null, "not_for_this_device"))
        assertEquals(KeyVerifyPlan.Outcome.UnknownDevice, KeyVerifyPlan.outcome(null, "unknown_device"))
        assertEquals(KeyVerifyPlan.Outcome.KeyChanged, KeyVerifyPlan.outcome(null, "key_changed"))
        assertEquals(KeyVerifyPlan.Outcome.Mismatch, KeyVerifyPlan.outcome(null, "mismatch"))
        assertEquals(KeyVerifyPlan.Outcome.NotACode, KeyVerifyPlan.outcome(null, "something_new"))
        // A problem always wins over a node id.
        assertEquals(KeyVerifyPlan.Outcome.Mismatch, KeyVerifyPlan.outcome("desk", "mismatch"))
    }

    @Test
    fun theSafetyNumberShowsInThreeLinesOfFour() {
        val n = "83209 04899 17154 39681 79009 07377 50615 33206 92703 06688 20318 89074"
        assertEquals(
            listOf("83209 04899 17154 39681", "79009 07377 50615 33206", "92703 06688 20318 89074"),
            KeyVerifyPlan.safetyLines(n),
        )
        assertEquals(emptyList<String>(), KeyVerifyPlan.safetyLines(""))
    }

    @Test
    fun theBannerWarnsWhenTheOldKeyWasVerified() {
        val c = KeyChangeInfo("phone", "Phone", "AAAA BBBB CCCC DDDD", "1111 2222 3333 4444")
        assertFalse(KeyVerifyPlan.warnVerified(c))
        assertTrue(KeyVerifyPlan.warnVerified(c.copy(wasVerified = true)))
    }
}
