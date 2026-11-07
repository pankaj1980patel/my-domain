package com.mydomain.android.core

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import org.json.JSONObject

/** One device in a passphrase change (the core's `RotationDevice`). */
data class RotationDeviceInfo(val nodeId: String, val name: String, val status: String)

/** The passphrase change, as the core reports it (`KeyRotation`). */
data class KeyRotationInfo(
    /** "none", "preparing", "switching", "done", "expired" or "cancelled". */
    val state: String = "none",
    val byMe: Boolean = false,
    val byName: String = "",
    val devices: List<RotationDeviceInfo> = emptyList(),
    val expiresInS: Long = 0,
    /** This phone missed a change: it needs the new passphrase typed in. */
    val needsPassphrase: Boolean = false,
    val mismatchName: String = "",
) {
    val active: Boolean get() = state == "preparing" || state == "switching"
}

/**
 * Changing the passphrase on all your devices (`features::rekey` in the
 * core): the new key goes to each trusted device sealed to that device;
 * Finish switches them all, Cancel (before Finish) drops it. Read on the
 * Settings and Home screens and again when the core says it moved on
 * (`key_rotation_changed`).
 */
object KeyRotation {
    var info by mutableStateOf(KeyRotationInfo())
        private set

    /** Read it again (off the main thread). */
    suspend fun load() {
        info = parse(JSONObject(Core.call { e -> e.keyRotation() }))
    }

    private suspend fun after(block: suspend () -> Unit) {
        try {
            block()
        } finally {
            runCatching { load() }
        }
    }

    /** Start the change. Throws the core's error. */
    suspend fun start(passphrase: String) = after { Core.call { e -> e.startKeyRotation(passphrase) } }

    suspend fun finish() = after { Core.call { e -> e.finishKeyRotation() } }

    suspend fun cancel() = after { Core.call { e -> e.cancelKeyRotation() } }

    suspend fun dismiss() = after { Core.call { e -> e.dismissKeyRotation() } }

    /** This phone missed it: the new passphrase, typed in. */
    suspend fun typed(passphrase: String) = after { Core.call { e -> e.setEncryptionKey(passphrase) } }

    /** 0 (too short) to 4. */
    suspend fun strength(passphrase: String): Int =
        Core.call { e -> e.passphraseStrength(passphrase).toInt() }

    fun parse(o: JSONObject): KeyRotationInfo {
        val list = o.optJSONArray("devices")
        val devices = if (list == null) emptyList() else (0 until list.length()).mapNotNull { i ->
            val d = list.optJSONObject(i) ?: return@mapNotNull null
            val node = d.optString("node_id")
            if (node.isEmpty()) null else RotationDeviceInfo(node, d.optString("name").ifEmpty { node }, d.optString("status"))
        }
        return KeyRotationInfo(
            state = o.optString("state", "none"),
            byMe = o.optBoolean("by_me"),
            byName = o.optString("by_name"),
            devices = devices,
            expiresInS = o.optLong("expires_in_s"),
            needsPassphrase = o.optBoolean("needs_passphrase"),
            mismatchName = o.optString("mismatch_name"),
        )
    }
}
