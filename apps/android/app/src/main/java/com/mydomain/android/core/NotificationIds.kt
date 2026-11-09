package com.mydomain.android.core

import java.util.concurrent.atomic.AtomicInteger

/**
 * Every notification id the app posts, in one place, so no two can meet.
 *
 * Android keeps one notification per id: two features sharing an id
 * replace (or cancel) each other's notice. That happened: camera sharing
 * and phone audio both used 7312 for their always-on notice (running both
 * broke their Stop buttons), three permission prompts used 7311, two used
 * 7321, and Lost mode's notice shared 7701 with the screen-settings prompt.
 *
 * - Fixed ids (one notice per feature) are the constants below, each
 *   unique and below [FAMILY_BASE] (NotificationIdsTest checks both).
 * - Notices of which there can be many (one per transfer, per device, per
 *   note...) get an id inside their own [Family] block of 2^24 ids:
 *   [of] for one per key (the same key replaces its notice), [next] for a
 *   new one each time. Blocks never overlap each other or the fixed ids.
 */
object NotificationIds {
    // --- fixed ---------------------------------------------------------------

    /** NetService's always-on notice. */
    const val SERVICE = 1
    /** A device rings this phone (find my phone). */
    const val RING = 2001

    const val SCREEN_SHARE_ASK = 7301
    const val SCREEN_SHARING = 7302
    const val PHONE_AUDIO_ASK = 7311
    const val PHONE_AUDIO_PLAYING = 7312
    const val COMPUTER_AUDIO_ASK = 7321
    const val COMPUTER_AUDIO_PLAYING = 7322
    const val CAMERA_ASK = 7331
    const val CAMERA_ON = 7332
    const val PEER_PERMISSION_ASK = 7341
    const val WIFI_DIRECT_ASK = 7351
    const val PRIVILEGED_ASK = 7361
    const val WIFI_NETWORKS = 7501
    const val LOCATION_ASK = 7601
    const val LOCATION_SHARED = 7602
    const val LOST_MODE = 7701
    const val SCREEN_SETTINGS_ASK = 7711
    const val SECURE_MODE_NEEDED = 7801

    // --- many of a kind ----------------------------------------------------------

    /** Ids at and above this belong to a [Family]. */
    const val FAMILY_BASE = 1 shl 24
    private const val MASK = FAMILY_BASE - 1

    /** A kind of notice there can be many of, with its own block of ids
     *  (`tag shl 24` up to the next block). Tags stay below 0x80, so every
     *  id is positive. */
    enum class Family(val tag: Int) {
        /** A notification another device shared. */
        SHARED(0x10),
        /** A link, app, text or media another device sent. */
        LINK(0x11),
        /** An automation's own notice. */
        AUTOMATION(0x12),
        /** An automation app's request. */
        AUTOMATION_APP(0x13),
        /** A received file, per transfer. */
        FILE(0x14),
        /** "Download on this phone?", per request. */
        DOWNLOAD_ASK(0x15),
        /** A device's battery alert, per device and kind. */
        BATTERY_ALERT(0x16),
        /** A folder sync invitation, per pair. */
        SYNC(0x17),
        /** A device came back with another key, per device. */
        KEY_CHANGE(0x18),
        /** A note's reminder, per note. */
        NOTE_REMINDER(0x19),
        /** A timer that went off, per timer. */
        TIMER_RING(0x1A),
        /** A running timer, per timer. */
        TIMER_RUNNING(0x1B),
        /** A computer's own notification, per computer and key. */
        COMPUTER_NOTIF(0x1C),
        /** The group summary of a computer's notifications, per computer. */
        COMPUTER_SUMMARY(0x1D),
        /** A computer asks for a photo or a scan, per request. */
        INSERT(0x1E),
    }

    private val counter = AtomicInteger(0)

    /** The id of [family]'s notice for [key] (the same key, the same id). */
    fun of(family: Family, key: String): Int = (family.tag shl 24) or (key.hashCode() and MASK)

    /** A new id in [family] (a notice that never replaces another). */
    fun next(family: Family): Int = (family.tag shl 24) or (counter.incrementAndGet() and MASK)

    /** Which family [id] is in (null: a fixed id). */
    fun familyOf(id: Int): Family? = if (id < FAMILY_BASE) null else Family.entries.firstOrNull { it.tag == id ushr 24 }
}
