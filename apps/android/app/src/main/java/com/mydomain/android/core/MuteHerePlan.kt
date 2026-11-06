package com.mydomain.android.core

/**
 * "Mute here" (the phone's own speaker while its audio plays on a
 * computer; MuteHere.kt does it), the part with no Android in it: when to
 * mute, and what to undo. The same rules as the desktop's `mute_here.rs`.
 *
 * What this app changed is written down before it changes it: "1" when it
 * muted the music stream, "0" when it found it muted already (nothing to
 * undo). Undo only what it did: a stream muted before, or unmuted by hand
 * since, is left alone. After a crash, the next start undoes it.
 */
object MuteHerePlan {
    /** Muting, with the stream [muted] now: what to set it to (null: leave
     *  it), and what to write down. */
    fun begin(muted: Boolean): Pair<Boolean?, String> =
        if (muted) null to WAS_MUTED else true to MUTED_BY_US

    /** Undoing, with what was written down ([saved], null: nothing) and the
     *  stream [muted] now: what to set it to (null: leave it). */
    fun end(saved: String?, muted: Boolean): Boolean? =
        if (saved == MUTED_BY_US && muted) false else null

    /** Mute while any computer this phone plays on has the switch on. */
    fun wanted(playingTo: Set<String>, switchOn: (String) -> Boolean): Boolean =
        playingTo.any(switchOn)

    const val MUTED_BY_US = "1"
    const val WAS_MUTED = "0"
}

/** Which way audio with a device goes (the core's `AudioRoute`). */
object AudioRoutes {
    const val AUTO = "auto"
    const val WIFI = "wifi"
    const val BLUETOOTH = "bluetooth"
    val ALL = listOf(AUTO, WIFI, BLUETOOTH)

    /** A saved route, or Auto for anything else. */
    fun of(v: String?): String = if (v in ALL) v!! else AUTO

    /** The "via Bluetooth" badge: the core said the frames went that way. */
    fun viaBluetooth(via: String?): Boolean = via == "bluetooth"
}
