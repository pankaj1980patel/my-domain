package com.mydomain.android.core

/**
 * Secure mode (SecureMode.kt does it): the phone helper, my-domain's
 * Accessibility service, switched off so banking and train-ticket apps
 * that refuse to run beside any Accessibility service work, and back on
 * when you want your computers to control the phone again. The part with
 * no Android in it: Android's list of enabled services, what the switch
 * shows, and when it turns itself on.
 */
object SecureModePlan {
    /** Android's `enabled_accessibility_services`: component names joined
     *  with ':'. Does [list] hold [component]? (Case-insensitive, as
     *  Android compares them; a short ".Class" form counts too.) */
    fun contains(list: String?, component: String): Boolean =
        entries(list).any { same(it, component) }

    /** [list] with [component] added at the end (once), every other
     *  service kept as it was. */
    fun add(list: String?, component: String): String {
        val now = entries(list)
        if (now.any { same(it, component) }) return now.joinToString(":")
        return (now + component).joinToString(":")
    }

    /** [list] without [component], every other service kept. */
    fun remove(list: String?, component: String): String =
        entries(list).filterNot { same(it, component) }.joinToString(":")

    private fun entries(list: String?): List<String> =
        list.orEmpty().split(':').map { it.trim() }.filter { it.isNotEmpty() }

    /** "pkg/.Cls" and "pkg/pkg.Cls" are the same service. */
    private fun same(a: String, b: String): Boolean = full(a).equals(full(b), ignoreCase = true)

    private fun full(c: String): String {
        val slash = c.indexOf('/')
        if (slash <= 0) return c
        val pkg = c.substring(0, slash)
        val cls = c.substring(slash + 1)
        return if (cls.startsWith(".")) "$pkg/$pkg$cls" else c
    }

    /** What the switch shows. */
    enum class State {
        /** The helper is on (Secure mode off). */
        HELPER_ON,
        /** Secure mode is on: the helper is off, as you asked. */
        SECURE,
        /** You turned Secure mode on and the helper is switching off. */
        TURNING_OFF,
        /** The helper is off, but not by Secure mode (never turned on, or
         *  off in Android's Settings). */
        HELPER_OFF,
    }

    /** From whether you asked for Secure mode ([wanted]) and whether the
     *  helper is on in Android's list ([enabled]). */
    fun state(wanted: Boolean, enabled: Boolean): State = when {
        wanted && enabled -> State.TURNING_OFF
        wanted -> State.SECURE
        enabled -> State.HELPER_ON
        else -> State.HELPER_OFF
    }

    /** The switch (and the tile) shows Secure mode on. */
    fun shownOn(state: State): Boolean = state == State.SECURE || state == State.TURNING_OFF

    /** How the helper gets back on when you turn Secure mode off. */
    enum class Path {
        /** On already: nothing to do. */
        NONE,
        /** This app may write Android's secure settings (you granted it
         *  over adb): on at once. */
        WRITE,
        /** Android's Accessibility settings open for you to turn it on. */
        SETTINGS,
    }

    fun enablePath(enabled: Boolean, canWriteSecure: Boolean): Path = when {
        enabled -> Path.NONE
        canWriteSecure -> Path.WRITE
        else -> Path.SETTINGS
    }

    /** "After N minutes without remote control": the choices (0 = never). */
    val IDLE_CHOICES = listOf(0, 5, 15, 60)

    /** A saved choice, or never for anything else. */
    fun idleMinutes(saved: Int): Int = if (saved in IDLE_CHOICES) saved else 0

    /** Should the helper turn Secure mode on now? [idleMin] minutes with no
     *  remote use since [lastUseMs] (its last tap, key or switch, or when
     *  it came on), never while a computer may control the screen
     *  ([controlling]). */
    fun idleDue(idleMin: Int, lastUseMs: Long, nowMs: Long, controlling: Boolean): Boolean =
        idleMin > 0 && !controlling && nowMs - lastUseMs >= idleMin * MINUTE_MS

    /** When to look again (ms from now), or null: never. */
    fun nextCheckMs(idleMin: Int, lastUseMs: Long, nowMs: Long, controlling: Boolean): Long? {
        if (idleMin <= 0) return null
        if (controlling) return MINUTE_MS
        val left = lastUseMs + idleMin * MINUTE_MS - nowMs
        return left.coerceIn(1_000L, idleMin * MINUTE_MS)
    }

    /** A remote-control session ended: turn Secure mode on if you asked for
     *  that, and control was actually allowed in it. */
    fun afterSessionDue(afterSession: Boolean, controlWasAllowed: Boolean): Boolean =
        afterSession && controlWasAllowed

    /** Say "Secure mode is on" at most this often (one per burst of taps). */
    fun shouldTell(lastToldMs: Long, nowMs: Long): Boolean = nowMs - lastToldMs >= TELL_EVERY_MS

    const val MINUTE_MS = 60_000L
    const val TELL_EVERY_MS = 30_000L
}
