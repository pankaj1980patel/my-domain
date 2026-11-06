package com.mydomain.android.core

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class ClipPrivacyTest {
    @Test
    fun aSensitiveCopyStaysHereWhileTheSettingIsOn() {
        assertTrue(ClipPrivacy.keepHere(sensitive = true, hide = true))
        assertFalse(ClipPrivacy.keepHere(sensitive = false, hide = true))
    }

    @Test
    fun withTheSettingOffEveryCopyGoes() {
        assertFalse(ClipPrivacy.keepHere(sensitive = true, hide = false))
        assertFalse(ClipPrivacy.keepHere(sensitive = false, hide = false))
    }

    @Test
    fun theExtraIsAndroidsOwnKey() {
        // ClipDescription.EXTRA_IS_SENSITIVE's value (API 33).
        assertTrue(ClipPrivacy.EXTRA_IS_SENSITIVE == "android.content.extra.IS_SENSITIVE")
    }
}
