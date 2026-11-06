package com.mydomain.android.core

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class NotifFilterTest {

    private fun facts(
        pkg: String = "com.whatsapp",
        title: String = "Asha",
        text: String = "Dinner at 8?",
        importance: Int = 4,
        category: String? = "msg",
        template: String? = null,
    ) = NotifFacts(
        pkg = pkg, ownPkg = "com.mydomain.android", importance = importance,
        category = category, template = template, title = title, text = text,
    )

    @Test
    fun anOrdinaryMessageGoes() {
        assertEquals(NotifSkip.NONE, NotifFilter.skip(facts()))
        // Importance unknown: it goes.
        assertEquals(NotifSkip.NONE, NotifFilter.skip(facts(importance = NotifFilter.IMPORTANCE_UNSPECIFIED)))
    }

    @Test
    fun noiseStaysOnThePhone() {
        assertEquals(NotifSkip.OWN, NotifFilter.skip(facts(pkg = "com.mydomain.android")))
        assertEquals(NotifSkip.ONGOING, NotifFilter.skip(facts().copy(ongoing = true)))
        assertEquals(NotifSkip.SUMMARY, NotifFilter.skip(facts().copy(groupSummary = true)))
        assertEquals(NotifSkip.FOREGROUND, NotifFilter.skip(facts().copy(foregroundService = true)))
        assertEquals(NotifSkip.LOCAL_ONLY, NotifFilter.skip(facts().copy(localOnly = true)))
        assertEquals(NotifSkip.LOW_IMPORTANCE, NotifFilter.skip(facts(importance = 2)))
        assertEquals(NotifSkip.LOW_IMPORTANCE, NotifFilter.skip(facts(importance = 1)))
        assertEquals(NotifSkip.MEDIA, NotifFilter.skip(facts(template = "android.app.Notification\$MediaStyle")))
        assertEquals(NotifSkip.MEDIA, NotifFilter.skip(facts(template = "android.app.Notification\$DecoratedMediaCustomViewStyle")))
        assertEquals(NotifSkip.SYSTEM, NotifFilter.skip(facts(category = "progress")))
        assertEquals(NotifSkip.SYSTEM, NotifFilter.skip(facts(category = "transport")))
        assertEquals(NotifSkip.SYSTEM, NotifFilter.skip(facts(pkg = "com.android.systemui")))
        assertEquals(NotifSkip.EMPTY, NotifFilter.skip(facts(title = " ", text = "")))
    }

    @Test
    fun moneyAndPasswordAppsAreSensitive() {
        assertTrue(NotifFilter.sensitiveApp("com.snapwork.hdfc", "HDFC Bank"))
        assertTrue(NotifFilter.sensitiveApp("com.sbi.lotusintouch", "YONO Lite"))
        assertTrue(NotifFilter.sensitiveApp("net.one97.paytm", "Paytm"))
        assertTrue(NotifFilter.sensitiveApp("com.phonepe.app", "PhonePe"))
        assertTrue(NotifFilter.sensitiveApp("com.google.android.apps.nbu.paisa.user", "Google Pay"))
        assertTrue(NotifFilter.sensitiveApp("com.google.android.apps.authenticator2", "Authenticator"))
        assertTrue(NotifFilter.sensitiveApp("com.x8bit.bitwarden", "Bitwarden"))
        assertTrue(NotifFilter.sensitiveApp("com.example.mybankapp", "Mine"))
        assertFalse(NotifFilter.sensitiveApp("com.whatsapp", "WhatsApp"))
        assertFalse(NotifFilter.sensitiveApp("com.spotify.music", "Spotify"))
        assertFalse(NotifFilter.sensitiveApp("com.google.android.gm", "Gmail"))
        assertFalse(NotifFilter.sensitiveApp("com.android.display", "Display"))
    }

    @Test
    fun aCodeOrASecretMakesItSensitive() {
        assertTrue(NotifFilter.isSensitive("com.google.android.apps.messaging", "Messages", "AX-HDFCBK",
            "482913 is your OTP for login. Do not share it.", 0))
        assertTrue(NotifFilter.isSensitive("com.whatsapp", "WhatsApp", "Asha", "hi", NotifFilter.VISIBILITY_SECRET))
        assertFalse(NotifFilter.isSensitive("com.whatsapp", "WhatsApp", "Asha", "Dinner at 8?", 0))
    }

    @Test
    fun theSameOneAgainIsARepeatAndAChangeIsAnUpdate() {
        val d = NotifDedupe(max = 2)
        val a = notifFingerprint("Asha", "hi", listOf("hi"))
        assertEquals(NotifDedupe.Seen.NEW, d.seen("k1", a))
        assertEquals(NotifDedupe.Seen.REPEAT, d.seen("k1", a))
        assertEquals(NotifDedupe.Seen.UPDATE, d.seen("k1", notifFingerprint("Asha", "hi", listOf("hi", "there"))))
        // Another key is another notification.
        assertEquals(NotifDedupe.Seen.NEW, d.seen("k2", a))
        // Forgotten (gone on the phone): new again.
        d.forget("k2")
        assertEquals(NotifDedupe.Seen.NEW, d.seen("k2", a))
        // The oldest go past the cap.
        d.seen("k3", a)
        assertEquals(NotifDedupe.Seen.NEW, d.seen("k1", a))
    }

    @Test
    fun fingerprintsTellPicturesAndMessagesApart() {
        assertTrue(notifFingerprint("a", "b") != notifFingerprint("a", "b", picture = true))
        assertTrue(notifFingerprint("a", "b") != notifFingerprint("a", "b", listOf("c")))
        assertEquals(notifFingerprint(" a", "b "), notifFingerprint("a", "b"))
    }
}
