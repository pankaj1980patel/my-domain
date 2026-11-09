package com.mydomain.android.core

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import java.lang.reflect.Modifier

class NotificationIdsTest {

    /** Every fixed id (the `const val`s), by name, read from the class so a
     *  new one can't be left out of the check. */
    private fun fixedIds(): Map<String, Int> =
        NotificationIds::class.java.declaredFields
            .filter { Modifier.isStatic(it.modifiers) && it.type == Int::class.javaPrimitiveType }
            .filter { it.name != "FAMILY_BASE" && it.name != "MASK" }
            .associate { it.isAccessible = true; it.name to it.getInt(null) }

    @Test
    fun fixedIdsAreUnique() {
        val ids = fixedIds()
        assertTrue("found the constants", ids.size >= 18)
        val dupes = ids.entries.groupBy { it.value }.filterValues { it.size > 1 }
        assertTrue("shared ids: $dupes", dupes.isEmpty())
    }

    @Test
    fun fixedIdsStayBelowTheFamilies() {
        fixedIds().forEach { (name, id) ->
            assertTrue("$name = $id", id in 1 until NotificationIds.FAMILY_BASE)
            assertNull(NotificationIds.familyOf(id))
        }
    }

    @Test
    fun theOnesThatClashedNoLongerDo() {
        // Camera sharing and phone audio both had 7312; Lost mode and the
        // screen-settings prompt both had 7701; three prompts had 7311.
        assertNotEquals(NotificationIds.CAMERA_ON, NotificationIds.PHONE_AUDIO_PLAYING)
        assertNotEquals(NotificationIds.LOST_MODE, NotificationIds.SCREEN_SETTINGS_ASK)
        val asks = setOf(
            NotificationIds.PHONE_AUDIO_ASK, NotificationIds.CAMERA_ASK, NotificationIds.PEER_PERMISSION_ASK,
            NotificationIds.COMPUTER_AUDIO_ASK, NotificationIds.WIFI_DIRECT_ASK,
        )
        assertEquals(5, asks.size)
    }

    @Test
    fun familyTagsAreUniqueAndKeepIdsPositive() {
        val tags = NotificationIds.Family.entries.map { it.tag }
        assertEquals(tags.size, tags.toSet().size)
        tags.forEach { assertTrue(it in 1 until 0x80) }
    }

    @Test
    fun eachFamilyStaysInItsBlock() {
        val keys = listOf("", "a", "phone", "node|key", "x".repeat(500), "\u0000", "Zz9")
        NotificationIds.Family.entries.forEach { f ->
            keys.forEach { k ->
                val id = NotificationIds.of(f, k)
                assertTrue(id > 0)
                assertEquals(f, NotificationIds.familyOf(id))
                assertEquals("same key, same id", id, NotificationIds.of(f, k))
            }
            repeat(3) { assertEquals(f, NotificationIds.familyOf(NotificationIds.next(f))) }
        }
    }

    @Test
    fun theSameKeyInTwoFamiliesGetsTwoIds() {
        assertNotEquals(
            NotificationIds.of(NotificationIds.Family.FILE, "t1"),
            NotificationIds.of(NotificationIds.Family.DOWNLOAD_ASK, "t1"),
        )
    }

    @Test
    fun newIdsDontRepeat() {
        val seen = (1..1000).map { NotificationIds.next(NotificationIds.Family.SHARED) }
        assertEquals(seen.size, seen.toSet().size)
    }
}
