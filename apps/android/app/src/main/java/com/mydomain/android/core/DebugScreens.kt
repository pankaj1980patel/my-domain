package com.mydomain.android.core

/**
 * Debug builds only (MainActivity checks FLAG_DEBUGGABLE): open one screen
 * straight from adb, for screenshots (scripts/screenshots-android.sh):
 *
 *   adb shell am start -n com.mydomain.android/.MainActivity \
 *       --es md_screen device --es md_device sample-macbook --ez md_sample true
 *
 * `md_screen` is one of [DebugScreen]'s keys (case, `-` and `_` don't
 * matter); `md_device` the device for `device`, `md_topic` a Help topic,
 * `md_query` what Search starts with. `md_sample` swaps in [SampleData]
 * (made-up devices, in memory only). Pure: tested on the JVM.
 */
enum class DebugScreen(val key: String) {
    HOME("home"),
    NOTES("notes"),
    ACTIVITY("activity"),
    SETTINGS("settings"),
    DEVICE("device"),
    DEVELOPER("developer"),
    AUTOMATIONS("automations"),
    TRANSFERS("transfers"),
    SETUP_PHONE("setup"),
    TIMERS("timers"),
    READ_LATER("readLater"),
    LINKS_MAP("linksMap"),
    HEALTH("health"),
    ONBOARDING("onboarding"),
    HELP("help"),
    SEARCH("search"),
    SIGN_IN("signIn"),
    DEVICES("devices"),
    MORE("more"),
    FILES("files"),
    MESSAGES("messages"),
}

/** A screen asked for from adb, with what it needs. */
data class ScreenRequest(
    val screen: DebugScreen,
    val device: String? = null,
    val topic: String? = null,
    val query: String? = null,
)

object DebugScreens {
    private fun norm(s: String) = s.lowercase().filter { it != '-' && it != '_' && !it.isWhitespace() }

    /** The screen named [screen], or null (none, unknown, or `device`
     *  without a [device]). */
    fun parse(screen: String?, device: String? = null, topic: String? = null, query: String? = null): ScreenRequest? {
        val name = screen?.let(::norm)?.takeIf { it.isNotEmpty() } ?: return null
        val which = DebugScreen.entries.firstOrNull { norm(it.key) == name || norm(it.name) == name } ?: return null
        val node = device?.trim()?.takeIf { it.isNotEmpty() }
        if (which == DebugScreen.DEVICE && node == null) return null
        return ScreenRequest(
            screen = which,
            device = node.takeIf { which == DebugScreen.DEVICE },
            topic = topic?.trim()?.takeIf { it.isNotEmpty() && which == DebugScreen.HELP },
            query = query?.takeIf { it.isNotBlank() && which == DebugScreen.SEARCH },
        )
    }
}
