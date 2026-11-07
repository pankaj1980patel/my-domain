package com.mydomain.android.core

/** One signed-in session of the account (the core's `AccountSession`). */
data class AccountSessionInfo(
    val id: String,
    /** The device that signed in with it; empty until it registered. */
    val name: String = "",
    val nodeId: String = "",
    /** Seconds since the epoch. */
    val created: Long = 0,
    val lastSeen: Long = 0,
    /** This phone's own session. */
    val current: Boolean = false,
)

/** What's wrong with the "Change account password" form, if anything. */
enum class PasswordFormError { NoCurrent, TooShort, Mismatch, SameAsCurrent }

/** How long ago something was, in words the UI picks a string for. */
sealed class Ago {
    object JustNow : Ago()
    data class Minutes(val n: Int) : Ago()
    data class Hours(val n: Int) : Ago()
    data class Days(val n: Int) : Ago()
}

/**
 * The account-security screens' decisions, kept free of Android so they
 * are unit-tested: checking the password form, naming a session, ordering
 * the list, and "5 min ago".
 */
object AccountPlan {
    /** Shortest account password the server takes. */
    const val MIN_PASSWORD = 6

    /** The first problem with the form, or null when it can be sent. */
    fun check(current: String, new: String, repeat: String): PasswordFormError? = when {
        current.isEmpty() -> PasswordFormError.NoCurrent
        new.length < MIN_PASSWORD -> PasswordFormError.TooShort
        new != repeat -> PasswordFormError.Mismatch
        new == current -> PasswordFormError.SameAsCurrent
        else -> null
    }

    /** The words for a session: its device's name, else null ("Unknown device"). */
    fun displayName(s: AccountSessionInfo, peerName: (String) -> String? = { null }): String? =
        s.name.trim().ifEmpty { null }
            ?: s.nodeId.takeIf { it.isNotEmpty() }?.let { peerName(it)?.takeIf { n -> n.isNotBlank() } }

    /** This phone first, then the most recently seen. */
    fun ordered(list: List<AccountSessionInfo>): List<AccountSessionInfo> =
        list.sortedWith(compareBy<AccountSessionInfo> { !it.current }.thenByDescending { it.lastSeen })

    /** Sessions that can be signed out from the list (not this phone's own). */
    fun others(list: List<AccountSessionInfo>): Int = list.count { !it.current }

    /** How long before [nowS] the time [atS] was (seconds since the epoch). */
    fun ago(atS: Long, nowS: Long): Ago {
        val d = (nowS - atS).coerceAtLeast(0)
        return when {
            d < 60 -> Ago.JustNow
            d < 3600 -> Ago.Minutes((d / 60).toInt())
            d < 86400 -> Ago.Hours((d / 3600).toInt())
            else -> Ago.Days((d / 86400).toInt())
        }
    }
}
