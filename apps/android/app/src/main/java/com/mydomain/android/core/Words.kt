package com.mydomain.android.core

import com.mydomain.android.ffi.MdException

// Everything the user reads about connections and errors, in plain words.
// No protocol names (QUIC, relay, NAT) on screen: those stay on the
// Developer screen. Mirrors apps/desktop/ui/src/lib/words.ts.

enum class LinkState { Direct, Relay, Connecting, Offline, Failed }

fun linkState(link: String?, progress: String?): LinkState = when {
    link == "relay" -> LinkState.Relay
    link != null -> LinkState.Direct
    progress == "failed" -> LinkState.Failed
    progress != null && progress != "connected" -> LinkState.Connecting
    else -> LinkState.Offline
}

fun linkStateOf(node: String): LinkState = linkState(Core.links[node], Core.progress[node])

fun linkLabel(s: LinkState): String = when (s) {
    LinkState.Direct -> "Connected directly"
    LinkState.Relay -> "Connected through the internet"
    LinkState.Connecting -> "Connecting…"
    LinkState.Failed -> "Couldn't reach it"
    LinkState.Offline -> "Not connected"
}

/** Link quality in plain words (the core's rating; no milliseconds here). */
fun qualityLabel(q: String?): String? = when (q) {
    "excellent" -> "Excellent connection"
    "good" -> "Good connection"
    "weak" -> "Weak connection"
    else -> null
}

/** One line under the status while the connect ladder runs. */
fun progressLabel(stage: String?): String? = when (stage) {
    "quic" -> "Looking for it on your networks…"
    "signaled_quic" -> "Asking it to connect back…"
    "relay" -> "Connecting through the internet…"
    "punching", "webrtc" -> "Finding a faster direct route…"
    "failed" -> "Make sure my-domain is open on it and it's online."
    else -> null
}

/** Turn a core error into something a person can act on. */
fun friendlyError(e: Throwable): String {
    when (e) {
        is MdException.NoKey -> return "Set your device key first (Settings)."
        is MdException.PeerNotFound -> return "That device isn't in your list any more."
        is MdException.NotLoggedIn -> return "You're signed out. Sign in again."
        is MdException.NotConnected -> return "Not connected yet. Connecting now: try again in a moment."
        else -> {}
    }
    val raw = e.message ?: e.toString()
    val m = raw.lowercase()
    return when {
        "connection closed" in m -> "The connection dropped. Reconnecting: try again in a moment."
        "invalid credentials" in m || "401" in m -> "Wrong username or password."
        "already exists" in m || "409" in m -> "That username is taken."
        "timed out" in m || "timeout" in m -> "That took too long. Check your internet connection."
        "dns" in m || "connection refused" in m || "network" in m -> "Can't reach the server. Check your internet connection."
        "not started" in m -> "my-domain is still starting. Try again in a moment."
        else -> raw
    }
}

/** "MacBook-Pro.local" -> "MacBook Pro"; keeps names readable. */
fun niceName(name: String): String =
    name.replace(Regex("\\.local$", RegexOption.IGNORE_CASE), "").replace(Regex("[-_]+"), " ").trim().ifEmpty { name }

/** First letter of the device's name, for its avatar. */
fun initial(name: String): String =
    (Regex("[A-Za-z0-9]").find(niceName(name))?.value ?: "?").uppercase()

fun timeAgo(ts: Long, now: Long = System.currentTimeMillis()): String {
    val s = ((now - ts) / 1000).coerceAtLeast(0)
    if (s < 10) return "just now"
    if (s < 60) return "${s}s ago"
    val m = Math.round(s / 60.0)
    if (m < 60) return "$m min ago"
    val h = Math.round(m / 60.0)
    if (h < 24) return "$h h ago"
    return java.text.DateFormat.getDateInstance().format(java.util.Date(ts))
}
