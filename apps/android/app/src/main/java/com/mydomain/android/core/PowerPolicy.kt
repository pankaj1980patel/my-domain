package com.mydomain.android.core

/**
 * Save battery: what heavy background work to hold back right now, and why
 * (the pure part of `PowerSaver`, which watches the phone and applies it).
 *
 * "Saving" (mode Automatic) while any of: Battery Saver is on, the battery
 * is at or below the threshold and not charging, or the phone is dozing
 * (device idle). Mode Always saves all the time; Off never does. While
 * saving, these wait: synced-folder rounds, the photo backup, sending
 * clipboard pictures by themselves, and restarting phone-audio or screen
 * sharing by itself.
 *
 * On a metered network (mobile data, a metered hotspot), unless the user
 * allowed "Use mobile data for big transfers", synced files over
 * [METERED_CAP] and the photo backup wait too, whatever the mode (that's
 * about data, not the battery).
 *
 * Never held back: messages, ring / find my phone, lost mode, notification
 * mirroring, call controls, reminders, and anything the user starts.
 */
object PowerPolicy {
    const val DEFAULT_THRESHOLD = 15
    const val MIN_THRESHOLD = 5
    const val MAX_THRESHOLD = 50
    /** Synced files bigger than this wait on a metered network. */
    const val METERED_CAP = 10L * 1024 * 1024

    /** The thresholds offered in Settings. */
    val THRESHOLDS = listOf(10, 15, 20, 30)

    enum class Mode(val key: String) {
        AUTO("auto"), ALWAYS("always"), OFF("off");

        companion object {
            fun of(key: String?): Mode = entries.firstOrNull { it.key == key } ?: AUTO
        }
    }

    /** The user's choices. */
    data class Settings(
        val mode: Mode = Mode.AUTO,
        val threshold: Int = DEFAULT_THRESHOLD,
        val mobileData: Boolean = false,
    )

    /** Where the phone stands. [level] null = not known yet. */
    data class Conditions(
        val powerSave: Boolean = false,
        val level: Int? = null,
        val charging: Boolean = false,
        val idle: Boolean = false,
        val metered: Boolean = false,
    )

    /** Why something waits. */
    enum class Reason { ALWAYS, BATTERY_SAVER, LOW_BATTERY, IDLE, METERED }

    /** What waits. */
    enum class Paused { FOLDER_SYNC, BIG_SYNC_FILES, PHOTO_BACKUP, CLIP_PICTURES, AUTO_RESTARTS }

    data class Plan(
        /** Saving battery: heavy work waits. */
        val saving: Boolean,
        /** Why, battery reasons first, then [Reason.METERED]. */
        val reasons: List<Reason>,
        /** What waits right now. */
        val paused: Set<Paused>,
        /** Synced files bigger than this (bytes) wait; 0 = no cap. */
        val meteredCap: Long,
        /** The photo backup may use the current network (it isn't
         *  metered, or mobile data is allowed). */
        val backupNetworkOk: Boolean = true,
    ) {
        /** Heavy background work may run (the core's switch). */
        val heavyAllowed: Boolean get() = !saving
        fun pauses(p: Paused) = p in paused

        companion object {
            val NONE = Plan(saving = false, reasons = emptyList(), paused = emptySet(), meteredCap = 0)
        }
    }

    /** Keep a threshold within what's offered. */
    fun clampThreshold(pct: Int): Int = pct.coerceIn(MIN_THRESHOLD, MAX_THRESHOLD)

    fun decide(s: Settings, c: Conditions): Plan {
        val reasons = mutableListOf<Reason>()
        when (s.mode) {
            Mode.OFF -> {}
            Mode.ALWAYS -> reasons += Reason.ALWAYS
            Mode.AUTO -> {
                if (c.powerSave) reasons += Reason.BATTERY_SAVER
                val low = c.level != null && c.level <= clampThreshold(s.threshold) && !c.charging
                if (low) reasons += Reason.LOW_BATTERY
                if (c.idle) reasons += Reason.IDLE
            }
        }
        val saving = reasons.isNotEmpty()
        val meteredRule = c.metered && !s.mobileData
        if (meteredRule) reasons += Reason.METERED
        val paused = buildSet {
            if (saving) {
                add(Paused.FOLDER_SYNC)
                add(Paused.PHOTO_BACKUP)
                add(Paused.CLIP_PICTURES)
                add(Paused.AUTO_RESTARTS)
            }
            if (meteredRule) {
                add(Paused.BIG_SYNC_FILES)
                add(Paused.PHOTO_BACKUP)
            }
        }
        return Plan(saving, reasons, paused, if (meteredRule) METERED_CAP else 0L, backupNetworkOk = !meteredRule)
    }
}
