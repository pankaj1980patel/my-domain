package com.mydomain.android.core

/**
 * Verifying a device's security key, the parts kept free of Android so
 * they are unit-tested: which links are verify codes, how the safety
 * number is laid out, what a scan's result says, and what the key-changed
 * banner warns about. The checks themselves are the core's
 * (`device_key::check_scan`): a code made for another device, from an
 * unknown one, or with keys that don't match is never marked verified.
 */
object KeyVerifyPlan {
    const val PREFIX = "mydomain://verify?"

    /** What a scanned verify code came to. */
    enum class Outcome { Verified, NotACode, NotForThisDevice, UnknownDevice, KeyChanged, Mismatch }

    /** A `mydomain://verify?…` link (as the camera app opens it). */
    fun isVerifyLink(s: String?): Boolean =
        s != null && s.trim().regionMatches(0, PREFIX, 0, PREFIX.length, ignoreCase = true)

    /** The core's answer: verified (a node id, no problem) or why not. */
    fun outcome(nodeId: String?, problem: String?): Outcome = when {
        problem == null && !nodeId.isNullOrEmpty() -> Outcome.Verified
        problem == "not_for_this_device" -> Outcome.NotForThisDevice
        problem == "unknown_device" -> Outcome.UnknownDevice
        problem == "key_changed" -> Outcome.KeyChanged
        problem == "mismatch" -> Outcome.Mismatch
        else -> Outcome.NotACode
    }

    /** The safety number as both devices show it: lines of four groups. */
    fun safetyLines(n: String): List<String> =
        n.trim().split(Regex("\\s+")).filter { it.isNotEmpty() }.chunked(4).map { it.joinToString(" ") }

    /** The key-changed banner warns harder when the old key was verified. */
    fun warnVerified(c: KeyChangeInfo): Boolean = c.wasVerified
}
