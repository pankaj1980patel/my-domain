package com.mydomain.android.core

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class AccountPlanTest {
    @Test
    fun theFormNeedsTheCurrentPasswordAndALongEnoughMatchingNewOne() {
        assertEquals(PasswordFormError.NoCurrent, AccountPlan.check("", "new-secret", "new-secret"))
        assertEquals(PasswordFormError.TooShort, AccountPlan.check("old", "short", "short"))
        assertEquals(PasswordFormError.Mismatch, AccountPlan.check("old", "new-secret", "new-secreT"))
        assertEquals(PasswordFormError.SameAsCurrent, AccountPlan.check("same-one", "same-one", "same-one"))
        assertNull(AccountPlan.check("old-one", "new-secret", "new-secret"))
        assertNull(AccountPlan.check("old-one", "123456", "123456"))
    }

    @Test
    fun aSessionIsNamedByItsDeviceThenTheLabelElseUnknown() {
        val s = AccountSessionInfo("1", name = " Pixel ", nodeId = "n1")
        assertEquals("Pixel", AccountPlan.displayName(s) { "Label" })
        assertEquals("Label", AccountPlan.displayName(s.copy(name = "")) { "Label" })
        assertNull(AccountPlan.displayName(s.copy(name = "")) { " " })
        assertNull(AccountPlan.displayName(AccountSessionInfo("2")) { "never asked" })
    }

    @Test
    fun thisPhoneComesFirstThenTheMostRecentlySeen() {
        val list = listOf(
            AccountSessionInfo("old", lastSeen = 10),
            AccountSessionInfo("new", lastSeen = 30),
            AccountSessionInfo("me", lastSeen = 5, current = true),
        )
        assertEquals(listOf("me", "new", "old"), AccountPlan.ordered(list).map { it.id })
        assertEquals(2, AccountPlan.others(list))
    }

    @Test
    fun agoIsBucketedAndNeverNegative() {
        val now = 1_000_000L
        assertEquals(Ago.JustNow, AccountPlan.ago(now - 30, now))
        assertEquals(Ago.JustNow, AccountPlan.ago(now + 100, now))
        assertEquals(Ago.Minutes(5), AccountPlan.ago(now - 300, now))
        assertEquals(Ago.Hours(2), AccountPlan.ago(now - 7200, now))
        assertEquals(Ago.Days(3), AccountPlan.ago(now - 3 * 86400, now))
    }
}
