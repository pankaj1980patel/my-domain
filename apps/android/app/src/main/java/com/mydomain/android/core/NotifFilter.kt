package com.mydomain.android.core

/**
 * Which of the phone's notifications are worth sending to a computer, which
 * hold a secret, and which are just the same one again. Pure Kotlin (no
 * Android types), so the rules are tested on the JVM; NotifListener reads
 * the facts off each StatusBarNotification.
 */

/** What the filter looks at in one notification. */
data class NotifFacts(
    val pkg: String,
    /** This app's own package. */
    val ownPkg: String,
    val ongoing: Boolean = false,
    val groupSummary: Boolean = false,
    val foregroundService: Boolean = false,
    /** Meant for this phone only (not bridged to watches and the like). */
    val localOnly: Boolean = false,
    /** The channel's importance (NotificationManager.IMPORTANCE_*), or
     *  [NotifFilter.IMPORTANCE_UNSPECIFIED] when unknown. */
    val importance: Int = NotifFilter.IMPORTANCE_UNSPECIFIED,
    val category: String? = null,
    /** Notification.EXTRA_TEMPLATE: the style's class name. */
    val template: String? = null,
    val title: String = "",
    val text: String = "",
)

/** Why a notification isn't sent ([NONE]: it is). */
enum class NotifSkip { NONE, OWN, ONGOING, SUMMARY, FOREGROUND, LOCAL_ONLY, LOW_IMPORTANCE, MEDIA, SYSTEM, EMPTY }

object NotifFilter {
    const val IMPORTANCE_UNSPECIFIED = -1000
    const val IMPORTANCE_DEFAULT = 3
    /** Notification.VISIBILITY_SECRET. */
    const val VISIBILITY_SECRET = -1

    /** Categories that are status, not news: playing, downloading,
     *  navigating, a running service... */
    private val NOISE_CATEGORIES = setOf(
        "service", "progress", "sys", "transport", "status", "navigation",
        "location_sharing", "stopwatch", "workout",
    )

    /** The system's own chatter. */
    private val SYSTEM_PKGS = setOf("android", "com.android.systemui", "com.android.shell")

    fun skip(f: NotifFacts): NotifSkip = when {
        f.pkg == f.ownPkg -> NotifSkip.OWN
        f.ongoing -> NotifSkip.ONGOING
        f.groupSummary -> NotifSkip.SUMMARY
        f.foregroundService -> NotifSkip.FOREGROUND
        f.localOnly -> NotifSkip.LOCAL_ONLY
        f.pkg in SYSTEM_PKGS -> NotifSkip.SYSTEM
        f.template?.endsWith("MediaStyle") == true || f.template?.endsWith("MediaCustomViewStyle") == true ->
            NotifSkip.MEDIA
        f.category in NOISE_CATEGORIES -> NotifSkip.SYSTEM
        f.importance != IMPORTANCE_UNSPECIFIED && f.importance < IMPORTANCE_DEFAULT -> NotifSkip.LOW_IMPORTANCE
        f.title.isBlank() && f.text.isBlank() -> NotifSkip.EMPTY
        else -> NotifSkip.NONE
    }

    /** Package name parts that mean money or passwords. */
    private val SENSITIVE_PARTS = setOf(
        "pay", "upi", "paisa", "paytm", "phonepe", "otp", "authenticator", "authenticator2", "authy",
        "bitwarden", "lastpass", "onepassword", "keepass", "keepassdx", "dashlane", "keeper", "vault",
        "imobile", "netbanking", "yono", "lotusintouch",
    )
    private val SENSITIVE_WORDS = Regex("""\b(bank|banking|wallet|authenticator|password|passwords|pay|upi)\b""", RegexOption.IGNORE_CASE)

    /** An app that deals in money or passwords, by its package or name. */
    fun sensitiveApp(pkg: String, label: String): Boolean {
        val parts = pkg.lowercase().split('.', '_')
        val byPkg = parts.any { p ->
            p in SENSITIVE_PARTS || "bank" in p || "wallet" in p || p.startsWith("pay") || p.endsWith("pay")
        }
        return byPkg || SENSITIVE_WORDS.containsMatchIn(label)
    }

    /** Whether to treat it as holding a secret: from such an app, marked
     *  secret by its app, or carrying a one-time code. */
    fun isSensitive(pkg: String, label: String, title: String, text: String, visibility: Int): Boolean =
        visibility == VISIBILITY_SECRET || sensitiveApp(pkg, label) || findCode("$title\n$text") != null
}

/** What a notification says, for telling an update from the same thing
 *  posted again. */
fun notifFingerprint(title: String, text: String, messages: List<String> = emptyList(), picture: Boolean = false): String =
    buildString {
        append(title.trim()).append('\u0001').append(text.trim())
        for (m in messages) append('\u0001').append(m.trim())
        if (picture) append("\u0001[picture]")
    }

/**
 * Remembers what each notification (by key) last said, so the same one
 * posted again with nothing new isn't sent again, and an update is known as
 * one. Memory only; the oldest go first past [max].
 */
class NotifDedupe(private val max: Int = 500) {
    private val last = object : LinkedHashMap<String, String>(16, 0.75f, true) {
        override fun removeEldestEntry(eldest: MutableMap.MutableEntry<String, String>?) = size > max
    }

    enum class Seen { NEW, UPDATE, REPEAT }

    @Synchronized
    fun seen(key: String, fingerprint: String): Seen {
        val before = last.put(key, fingerprint)
        return when (before) {
            null -> Seen.NEW
            fingerprint -> Seen.REPEAT
            else -> Seen.UPDATE
        }
    }

    @Synchronized
    fun forget(key: String) {
        last.remove(key)
    }
}
