package com.mydomain.android.core

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import org.json.JSONArray

/**
 * Account security (`engine::account` in the core): the account password,
 * "Sign out all devices", the signed-in sessions, and being signed out by
 * the server (password changed or "Sign out all devices" on another
 * device), which shows "Sign in again" instead of failing silently.
 */
object Account {
    /** The server signed this phone out; it hasn't signed in again. */
    var signedOut by mutableStateOf(false)
        private set

    var sessions by mutableStateOf<List<AccountSessionInfo>>(emptyList())
        private set

    /** Why the list couldn't be read (e.g. an older server), or null. */
    var sessionsError by mutableStateOf<String?>(null)
        private set

    /** Re-read whether the server signed this phone out (Core.reload, the event). */
    fun refreshSignedOut() {
        val e = Core.engine() ?: return
        val v = runCatching { e.signedOutElsewhere() }.getOrDefault(false)
        Core.onMain { signedOut = v }
    }

    /** The core said the server no longer takes this phone's session. */
    fun onSignedOut() {
        Core.onMain { signedOut = true }
        Core.reload()
    }

    /** Read the sessions list (off the main thread). */
    suspend fun loadSessions() {
        try {
            val list = parse(Core.call { e -> e.accountSessions() })
            sessions = AccountPlan.ordered(list)
            sessionsError = null
        } catch (e: Exception) {
            sessionsError = friendlyError(e)
        }
    }

    suspend fun signOutSession(id: String) {
        try {
            Core.call { e -> e.signOutSession(id) }
        } finally {
            refreshSignedOut()
            if (!signedOut) loadSessions()
            Core.reload()
        }
    }

    /** Change the account password. Throws the core's error. */
    suspend fun changePassword(current: String, new: String) {
        Core.call { e -> e.changeAccountPassword(current, new) }
        Core.reload()
    }

    /** Sign out every device; true when this phone signed straight back in. */
    suspend fun signOutAll(password: String): Boolean {
        try {
            return Core.call { e -> e.signOutAllDevices(password) }
        } finally {
            refreshSignedOut()
            Core.reload()
        }
    }

    /** After a sign-out by the server: the password alone signs back in. */
    suspend fun signInAgain(password: String) {
        Core.call { e -> e.signInAgain(password) }
        Core.onMain { signedOut = false }
        Core.reload()
    }

    /** "Sign in another way": the full sign-in screen. */
    suspend fun startOver() {
        Core.signOut()
        Core.onMain { signedOut = false }
    }

    private fun parse(json: String): List<AccountSessionInfo> {
        val a = JSONArray(json)
        return (0 until a.length()).mapNotNull { i ->
            val o = a.optJSONObject(i) ?: return@mapNotNull null
            val id = o.optString("id")
            if (id.isEmpty()) null else AccountSessionInfo(
                id = id,
                name = o.optString("name"),
                nodeId = o.optString("node_id"),
                created = o.optDouble("created", 0.0).toLong(),
                lastSeen = o.optDouble("last_seen", 0.0).toLong(),
                current = o.optBoolean("current"),
            )
        }
    }
}
