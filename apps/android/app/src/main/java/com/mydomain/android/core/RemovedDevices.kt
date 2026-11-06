package com.mydomain.android.core

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import org.json.JSONArray

/** A removed device, as the core lists it (`RemovedDevice`). */
data class RemovedDeviceInfo(
    val nodeId: String,
    val name: String,
    val fingerprint: String,
    val removedAgoS: Long,
    val byMe: Boolean,
    val byName: String,
)

/**
 * Devices removed for good (`features::revoke` in the core): a signed
 * revocation every device of yours keeps, so a removed device that still
 * has the passphrase isn't found and linked again. "Remove device" on a
 * device's screen writes one; "Allow again" in Settings takes it back.
 * The list ([list]) is read on the Settings screen and again when the core
 * says it changed (`revocations_changed`, from this or another device).
 */
object RemovedDevices {
    var list by mutableStateOf<List<RemovedDeviceInfo>>(emptyList())
        private set

    /** Read the list again (off the main thread). */
    suspend fun load() {
        val json = Core.call { e -> e.removedDevices() }
        list = parse(JSONArray(json))
    }

    /** Remove [node] for good. Throws the core's error. */
    suspend fun remove(node: String) {
        try {
            Core.call { e -> e.removeDevice(node) }
        } finally {
            runCatching { load() }
        }
    }

    /** "Allow again". Throws the core's error. */
    suspend fun allow(node: String) {
        try {
            Core.call { e -> e.allowDeviceAgain(node) }
        } finally {
            runCatching { load() }
        }
    }

    fun parse(a: JSONArray): List<RemovedDeviceInfo> =
        (0 until a.length()).mapNotNull { i ->
            val o = a.optJSONObject(i) ?: return@mapNotNull null
            val node = o.optString("node_id")
            if (node.isEmpty()) return@mapNotNull null
            RemovedDeviceInfo(
                nodeId = node,
                name = o.optString("name").ifEmpty { node },
                fingerprint = o.optString("fingerprint"),
                removedAgoS = o.optLong("removed_ago_s"),
                byMe = o.optBoolean("by_me"),
                byName = o.optString("by_name"),
            )
        }
}
