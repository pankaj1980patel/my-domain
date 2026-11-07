package com.mydomain.android.core

/**
 * Per-device permissions (mdcore `features::perms`), the parts that are
 * plain logic: which asks this phone shows, what an answer leads to, and
 * how often a refusal is said. Tested on the JVM.
 *
 *  - A computer may see this screen only if allowed for it ("screen"):
 *    always (the switch on its page), or once (your answer to its ask, or
 *    you sharing with it from its page). Android still asks every time.
 *  - "control" and "kvm" are a computer's: a phone never plays remote input
 *    or shares mouse and keyboard, so it never asks for them.
 *  - When a computer refuses what this phone asked (its touchpad, its
 *    screen), the phone says why, once in a while, not at every touch.
 */
object PermsPlan {
    const val CONTROL = "control"
    const val SCREEN = "screen"
    const val KVM = "kvm"

    const val ONCE = "once"
    const val ALWAYS = "always"
    const val DENY = "deny"

    /** A refusal from the same computer for the same thing is said at most
     *  this often. */
    const val SAY_EVERY_MS = 8_000L

    fun known(perm: String): Boolean = perm == CONTROL || perm == SCREEN || perm == KVM

    /** This phone shows the owner an ask for [perm]. */
    fun asksHere(perm: String): Boolean = perm == SCREEN

    /** After [answer] to an ask for [perm]: start Android's own prompt. */
    fun startsCapture(perm: String, answer: String): Boolean =
        perm == SCREEN && (answer == ONCE || answer == ALWAYS)

    /** You share this screen with a computer from its page: an "allow
     *  once" for it first, unless it's allowed always already. */
    fun shareAnswer(allowedAlways: Boolean): String? = if (allowedAlways) null else ONCE

    /** Says a refusal at most every [SAY_EVERY_MS] per computer and
     *  permission. */
    class Throttle(private val everyMs: Long = SAY_EVERY_MS) {
        private val last = HashMap<String, Long>()

        @Synchronized
        fun due(from: String, perm: String, nowMs: Long): Boolean {
            val key = "$from|$perm"
            val at = last[key]
            if (at != null && nowMs - at < everyMs && nowMs >= at) return false
            if (last.size > 256) last.clear()
            last[key] = nowMs
            return true
        }
    }
}
