package com.mydomain.android.core

import com.mydomain.android.R
import com.mydomain.android.core.Core.plural
import com.mydomain.android.core.Core.str
import com.mydomain.android.ffi.MdException

// Everything the user reads about connections and errors, in plain words.
// No protocol names (QUIC, relay, NAT) on screen: those stay on the
// Developer screen. Mirrors apps/desktop/ui/src/lib/words.ts.

enum class LinkState { Direct, Relay, Connecting, Offline, Failed }

fun linkState(link: String?, progress: String?): LinkState = when {
    link == "relay" -> LinkState.Relay
    link != null -> LinkState.Direct
    progress == "failed" -> LinkState.Failed
    // Didn't answer its push, or closed its link to save battery: not
    // connecting, and not broken either (it wakes when it's needed).
    progress == "asleep" -> LinkState.Offline
    progress != null && progress != "connected" -> LinkState.Connecting
    else -> LinkState.Offline
}

fun linkStateOf(node: String): LinkState = linkState(Core.links[node], Core.progress[node])

fun linkLabel(s: LinkState): String = when (s) {
    LinkState.Direct -> str(R.string.link_direct)
    LinkState.Relay -> str(R.string.link_relay)
    LinkState.Connecting -> str(R.string.link_connecting)
    LinkState.Failed -> str(R.string.link_failed)
    LinkState.Offline -> str(R.string.link_offline)
}

/** Link quality in plain words (the core's rating; no milliseconds here). */
fun qualityLabel(q: String?): String? = when (q) {
    "excellent" -> str(R.string.quality_excellent)
    "good" -> str(R.string.quality_good)
    "weak" -> str(R.string.quality_weak)
    else -> null
}

/** One line under the status while the connect ladder runs. */
fun progressLabel(stage: String?): String? = when (stage) {
    "quic" -> str(R.string.progress_quic)
    "signaled_quic" -> str(R.string.progress_signaled)
    "relay" -> str(R.string.progress_relay)
    "punching", "webrtc" -> str(R.string.progress_punching)
    "failed" -> str(R.string.progress_failed)
    "waking" -> str(R.string.progress_waking)
    "asleep" -> str(R.string.progress_asleep)
    else -> null
}

/** Turn a core error into something a person can act on: never the
 *  core's own text (that is logged instead). */
fun friendlyError(e: Throwable): String {
    when (e) {
        is MdException.NoKey -> return str(R.string.error_no_key)
        is MdException.PeerNotFound -> return str(R.string.error_peer_not_found)
        is MdException.NotLoggedIn -> return str(R.string.error_not_logged_in)
        is MdException.NotConnected -> return str(R.string.error_not_connected)
        else -> {}
    }
    val raw = e.message ?: e.toString()
    val res = ErrorWords.resOf(raw)
    if (res == ErrorWords.FALLBACK) runCatching { android.util.Log.w("friendlyError", raw, e) }
    return str(res)
}

/** Which plain-words message a core error's text gets (the pure part of
 *  [friendlyError], unit-tested). Anything not known: [FALLBACK]. */
object ErrorWords {
    val FALLBACK = R.string.error_generic

    fun resOf(raw: String): Int {
        val m = raw.lowercase()
        return when {
            "update the server to use qr join" in m -> R.string.error_update_server
            "update the server to list sessions" in m -> R.string.error_update_server_sessions
            "incorrect password" in m -> R.string.error_wrong_password
            "expired or was already used" in m -> R.string.error_join_code_used
            "too many tries" in m -> R.string.error_too_many_tries
            "join code is damaged" in m || "isn't a my-domain join code" in m -> R.string.error_bad_join_link
            "device key didn't come through" in m -> R.string.error_join_no_key
            "6-digit code" in m -> R.string.signin_join_missing
            "connection closed" in m -> R.string.error_connection_closed
            "invalid credentials" in m || "401" in m -> R.string.error_bad_credentials
            "already exists" in m || "409" in m -> R.string.error_username_taken
            "timed out" in m || "timeout" in m -> R.string.error_timeout
            "dns" in m || "connection refused" in m || "network" in m || "unreachable" in m -> R.string.error_unreachable
            "not started" in m -> R.string.error_not_started
            else -> FALLBACK
        }
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
    if (s < 10) return str(R.string.time_just_now)
    if (s < 60) return plural(R.plurals.time_seconds_ago, s.toInt(), s)
    val m = Math.round(s / 60.0)
    if (m < 60) return plural(R.plurals.time_minutes_ago, m.toInt(), m)
    val h = Math.round(m / 60.0)
    if (h < 24) return plural(R.plurals.time_hours_ago, h.toInt(), h)
    return java.text.DateFormat.getDateInstance().format(java.util.Date(ts))
}
