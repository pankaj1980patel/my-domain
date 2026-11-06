package com.mydomain.android.core

/**
 * Wi-Fi Direct for big files, the rules that need no phone (plain Kotlin,
 * so JVM tests cover them; the framework calls are in `WifiDirect.kt`).
 *
 * The core (mdcore `features::wifi_direct`) decides when to try it (a
 * transfer of at least [MIN_BYTES] still to go, not over the same local
 * network, a device that says it can) and who hosts the group; it asks this
 * phone to "host", "join" or "leave". [chooseRole] and [worthIt] are the
 * same rules, for what the screens say and for the log.
 */
object WifiDirectPlan {
    /** It can host a group (the group owner). */
    const val CAN_HOST = 1L
    /** It can join a group another device hosts (by its name and
     *  passphrase: Android 10 or later). */
    const val CAN_JOIN = 2L
    /** It can start a local-only hotspot a computer joins (Android 8+). */
    const val CAN_HOTSPOT = 4L
    /** A computer that joins a phone's hotspot (never a phone). */
    const val CAN_JOIN_HOTSPOT = 8L
    /** The smallest transfer that tries it (bytes still to send). */
    const val MIN_BYTES = 20L * 1024 * 1024
    /** How long hosting or joining may take before the core gives up. */
    const val SETUP_MS = 15_000L

    /** What this phone can do: Wi-Fi Direct hosting (and joining by name
     *  and passphrase from Android 10), and a local-only hotspot (Android
     *  8+, when the phone has one). */
    fun caps(sdk: Int, hasWifiDirect: Boolean, hasHotspot: Boolean = false): Long {
        var c = when {
            !hasWifiDirect -> 0L
            sdk >= 29 -> CAN_HOST or CAN_JOIN
            else -> CAN_HOST
        }
        if (hasHotspot && sdk >= 26) c = c or CAN_HOTSPOT
        return c
    }

    enum class Method { WifiDirect, Hotspot }

    /** The route, as the core picks it: Wi-Fi Direct when both can (it
     *  leaves everyone's Wi-Fi alone), else the phone's hotspot. */
    fun chooseMethod(me: String, myCaps: Long, peer: String, peerCaps: Long): Pair<Method, Role>? {
        chooseRole(me, myCaps, peer, peerCaps)?.let { return Method.WifiDirect to it }
        fun can(caps: Long, bit: Long) = caps and bit != 0L
        val iHost = can(myCaps, CAN_HOTSPOT) && can(peerCaps, CAN_JOIN_HOTSPOT)
        val theyHost = can(peerCaps, CAN_HOTSPOT) && can(myCaps, CAN_JOIN_HOTSPOT)
        return when {
            iHost && theyHost -> Method.Hotspot to (if (me < peer) Role.Host else Role.Join)
            iHost -> Method.Hotspot to Role.Host
            theyHost -> Method.Hotspot to Role.Join
            else -> null
        }
    }

    /** This phone's address on its hotspot, from its interfaces ([ifaces]:
     *  name to IPv4 address): one named like a hotspot (`swlan0`, `ap0`,
     *  `softap0`, `wlan1`…), not the address it has on a Wi-Fi it's on
     *  ([staIp]). Null when it can't tell (the computer asks its gateway). */
    fun pickHotspotAddress(ifaces: List<Pair<String, String>>, staIp: String?): String? {
        val apNames = listOf("swlan", "ap", "softap", "wlan1", "wlan2", "wigig")
        return ifaces.firstOrNull { (name, ip) ->
            ip != staIp && ip.isNotEmpty() && !ip.startsWith("127.") &&
                apNames.any { name.startsWith(it) }
        }?.second
    }

    enum class Role { Host, Join }

    /** Who hosts, as the core decides it on both ends: a device that can
     *  only host (a Windows PC, an older phone) does; between two that can
     *  do both, the lower node id. Null: they can't make a group. */
    fun chooseRole(me: String, myCaps: Long, peer: String, peerCaps: Long): Role? {
        fun can(caps: Long, bit: Long) = caps and bit != 0L
        val iHost = can(myCaps, CAN_HOST) && can(peerCaps, CAN_JOIN)
        val theyHost = can(peerCaps, CAN_HOST) && can(myCaps, CAN_JOIN)
        return when {
            iHost && theyHost -> if (me < peer) Role.Host else Role.Join
            iHost -> Role.Host
            theyHost -> Role.Join
            else -> null
        }
    }

    /** A transfer with [left] bytes to go is big enough. */
    fun worthIt(left: Long, minBytes: Long = MIN_BYTES): Boolean = left >= minBytes

    /** The permission Wi-Fi Direct needs: "nearby Wi-Fi devices" for an app
     *  that targets Android 13 on Android 13 or later, else location. */
    fun permissionsFor(sdk: Int, targetSdk: Int): List<String> =
        if (sdk >= 33 && targetSdk >= 33) listOf("android.permission.NEARBY_WIFI_DEVICES")
        else listOf("android.permission.ACCESS_FINE_LOCATION")

    /** Missing the permission: explain and ask only the first time; after
     *  that (or a no) the transfer just keeps its usual route. */
    fun shouldAsk(granted: Boolean, askedBefore: Boolean): Boolean = !granted && !askedBefore

    /** Where this phone is with one request of the core's. */
    enum class Phase { Idle, Hosting, Hosted, Joining, Joined, Failed, Left }

    /** What happens to it. */
    enum class Event { Host, Join, GroupUp, Connected, Failure, Leave }

    /** The next phase, or null when [event] doesn't fit [phase] (it's
     *  ignored: e.g. a late success after the core said leave). */
    fun next(phase: Phase, event: Event): Phase? = when (event) {
        Event.Host -> if (phase == Phase.Idle) Phase.Hosting else null
        Event.Join -> if (phase == Phase.Idle) Phase.Joining else null
        Event.GroupUp -> if (phase == Phase.Hosting) Phase.Hosted else null
        Event.Connected -> if (phase == Phase.Joining) Phase.Joined else null
        Event.Failure -> if (phase == Phase.Hosting || phase == Phase.Joining) Phase.Failed else null
        Event.Leave -> if (phase == Phase.Left) null else Phase.Left
    }

    /** Leaving needs the group removed (the phone made or joined one). */
    fun removeOnLeave(phase: Phase): Boolean =
        phase == Phase.Hosting || phase == Phase.Hosted || phase == Phase.Joining || phase == Phase.Joined

    /** A group's name and passphrase as Android takes them: `DIRECT-xy…`,
     *  at most 32 characters, and 8 to 63 for the passphrase. */
    fun validGroup(ssid: String, passphrase: String): Boolean =
        ssid.startsWith("DIRECT-") && ssid.length in 9..32 && passphrase.length in 8..63
}
