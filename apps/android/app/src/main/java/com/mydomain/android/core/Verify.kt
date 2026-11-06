package com.mydomain.android.core

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateMapOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import com.mydomain.android.ffi.KeyVerify
import com.mydomain.android.ffi.VerifyResult

/**
 * Verifying a device's security key: this phone's own fingerprint (Settings,
 * Add a device), and per device the fingerprints, safety number and QR code
 * to compare (the core's `key_verification`). "Verified" is kept by the
 * core with the key it was for, so trusting a new key clears it. A verify
 * code scanned with the camera app comes in as a `mydomain://verify?…`
 * link ([pendingScan], from MainActivity) and is checked by the core.
 */
object Verify {
    /** This phone's short fingerprint ("" until read). */
    var ownFingerprint by mutableStateOf("")
        private set

    /** Verified or not, per device, as last read. */
    val verified = mutableStateMapOf<String, Boolean>()

    /** A scanned verify link waiting to be checked (shown by VerifyScanResult). */
    var pendingScan by mutableStateOf<String?>(null)

    suspend fun loadOwn() {
        ownFingerprint = Core.call { e -> e.ownFingerprint() }
    }

    /** What to compare with [node]; null until it has connected with a key. */
    suspend fun load(node: String): KeyVerify? {
        val v = Core.call { e -> e.keyVerification(node) }
        verified[node] = v?.verified == true
        return v
    }

    /** Mark [node] verified (they match) or not. Throws the core's error. */
    suspend fun set(node: String, on: Boolean) {
        try {
            Core.call { e -> e.setDeviceVerified(node, on) }
        } finally {
            runCatching { load(node) }
        }
    }

    /** Check a scanned code; a device verified by it is marked so here too. */
    suspend fun scanned(text: String): VerifyResult {
        val r = Core.call { e -> e.verifyScanned(text) }
        r.nodeId?.let { if (r.problem == null) verified[it] = true }
        return r
    }
}
