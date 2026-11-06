package com.mydomain.android.core

/** A device whose security key changed (the core's `KeyChange`). */
data class KeyChangeInfo(
    val nodeId: String,
    val name: String,
    /** Short fingerprints ("1A2B 3C4D 5E6F 7A8B") of the key it had and the one it came with. */
    val oldFingerprint: String,
    val newFingerprint: String,
    val firstAgoS: Long = 0,
    val lastAgoS: Long = 0,
    val attempts: Int = 0,
    /** The user said "Not now": still refused, not asked again. */
    val dismissed: Boolean = false,
    /** The key it had was verified by the user: be careful. */
    val wasVerified: Boolean = false,
)

/**
 * Which key change to show where, kept free of Android so it is
 * unit-tested. The device's screen asks about a change not dismissed; the
 * connection map shows it either way (the device is refused until the user
 * trusts the new key). Trusting always names the fingerprint shown, so a
 * key that turned up since is never trusted by mistake.
 */
object KeyChangePlan {
    /** The change to show for [node]; dismissed ones only when [showDismissed]. */
    fun banner(list: List<KeyChangeInfo>, node: String, showDismissed: Boolean = false): KeyChangeInfo? =
        list.firstOrNull { it.nodeId == node && (showDismissed || !it.dismissed) }

    /** One notification per device: a newer key replaces the older one's. */
    fun notificationId(node: String): Int = 0x6000_0000 or (node.hashCode() and 0x0FFF_FFFF)

    /** The words to show for a device: its name, else its id. */
    fun nameOf(c: KeyChangeInfo, peerName: (String) -> String?): String =
        peerName(c.nodeId)?.takeIf { it.isNotBlank() } ?: c.name.ifBlank { c.nodeId }
}
