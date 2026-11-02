package com.mydomain.android

import com.mydomain.android.ffi.EventListener
import com.mydomain.android.ffi.MdEngine
import com.mydomain.android.ffi.MdException
import com.mydomain.android.ffi.runningEngine
import org.json.JSONArray
import org.json.JSONObject

/**
 * Transitional facade over the generated UniFFI API ([MdEngine], package
 * `com.mydomain.android.ffi`), keeping the string-based signatures the
 * current screens were written against: "" on success, "ERROR: …" on
 * failure, JSON for structured results.
 *
 * There is no JNI behind this any more. New code (the rebuilt UI) should call
 * [MdEngine] directly: typed results, exceptions, and pushed events.
 */
object RustNet {
    private fun engine(): MdEngine? = runningEngine()

    /** Run [block] on the engine; "" on success, "ERROR: …" otherwise. */
    private inline fun call(block: (MdEngine) -> Unit): String {
        val e = engine() ?: return "ERROR: not started"
        return try {
            block(e)
            ""
        } catch (x: MdException) {
            "ERROR: ${x.message}"
        }
    }

    /** Start networking (or attach to the running engine). Returns identity JSON. */
    fun nativeStart(deviceId: String, name: String, wifiIp: String, dataDir: String, listener: EventListener): String =
        try {
            val id = MdEngine.start(deviceId, name, wifiIp, dataDir, listener).identity()
            JSONObject().put("node_id", id.nodeId).put("name", id.name).put("ip", id.ip)
                .put("quic_port", id.quicPort.toInt()).toString()
        } catch (x: MdException) {
            "ERROR: ${x.message}"
        }

    fun nativeAuthLogin(serverUrl: String, username: String, password: String) = call { it.authLogin(serverUrl, username, password) }
    fun nativeAuthRegister(serverUrl: String, username: String, password: String) = call { it.authRegister(serverUrl, username, password) }
    fun nativeSetEncryptionKey(passphrase: String) = call { it.setEncryptionKey(passphrase) }
    fun nativeUpdateEncryptionKey(newPassphrase: String, password: String) = call { it.updateEncryptionKey(newPassphrase, password) }
    fun nativeGenerateKey(): String = com.mydomain.android.ffi.generateKey()
    /** DEV / LAN-only: set a local username + encryption key, skipping the registry login. */
    fun nativeDevLogin(username: String, passphrase: String) = call { it.devLogin(username, passphrase) }
    fun nativeLogout() = call { it.logout() }
    /** "true" / "false". */
    fun nativeIsReady(): String = (engine()?.isReady() ?: false).toString()

    /** Register this device + fetch peers from the registry. */
    fun nativeRefresh() = call { it.refresh() }
    fun nativeNetworkChanged(wifiIp: String) = call { it.networkChanged(wifiIp) }
    fun nativeScanLan() = call { it.scanLan() }

    /** Sends over the live link, opening a QUIC link first if needed.
     *  Returns the transport used on success, or "ERROR: …". */
    fun nativeSend(nodeId: String, text: String): String {
        val e = engine() ?: return "ERROR: not started"
        return try { e.send(nodeId, text) } catch (x: MdException) { "ERROR: ${x.message}" }
    }

    /** JSON array of peers (the fields the current screens read). */
    fun nativeGetPeers(): String {
        val arr = JSONArray()
        engine()?.peers()?.forEach { p ->
            arr.put(JSONObject().put("node_id", p.nodeId).put("name", p.name).put("ip", p.ip)
                .put("quic_port", p.quicPort.toInt()).put("source", p.source)
                .put("tcp_port", 0).put("udp_port", 0).put("ws_port", 0))
        }
        return arr.toString()
    }

    /** JSON array of node_ids with a live link. */
    fun nativeConnectedPeers(): String =
        JSONArray(engine()?.peers()?.filter { it.connected }?.map { it.nodeId } ?: emptyList<String>()).toString()

    // ---- feature sharing ----
    fun nativeShareNotification(title: String, body: String, app: String) =
        call { it.shareNotification(title, body, app.ifEmpty { null }) }
    fun nativeShareCallNotification(caller: String, number: String, state: String) =
        call { it.shareCallNotification(caller, number.ifEmpty { null }, state) }
    fun nativeShareCallHistory(entriesJson: String) = call { it.shareCallHistory(entriesJson) }

    // ---- signaling / connection setup ----
    fun nativeSetFcmToken(token: String): String { com.mydomain.android.ffi.setFcmToken(token); return "" }
    fun nativeOnSignal(from: String, payload: String) = call { it.onSignal(from, payload) }
    fun nativeConnect(nodeId: String) = call { it.connect(nodeId) }
    /** JSON {outbound_ok, inbound_blocked} or "ERROR: ...". */
    fun nativeFirewallCheck(): String {
        val e = engine() ?: return "ERROR: not started"
        return try {
            val fs = e.firewallCheck()
            JSONObject().put("outbound_ok", fs.outboundOk).put("inbound_blocked", fs.inboundBlocked).toString()
        } catch (x: MdException) { "ERROR: ${x.message}" }
    }

    // ---- app-notification pub/sub ----
    /** Publish this device's shareable app list (JSON array of {pkg,label}). */
    fun nativeSetInstalledApps(appsJson: String) = call { it.setInstalledApps(appsJson) }
    /** Consumer: request a producer's app list (reply arrives as an apps_list event). */
    fun nativeRequestApps(nodeId: String) = call { it.requestApps(nodeId) }
    /** Consumer: set enabled packages (JSON array of pkg strings) on a producer. */
    fun nativeSubscribeApps(nodeId: String, appsJson: String) = call { it.subscribeApps(nodeId, appsJson) }
    /** Producer: forward a captured app notification to subscribers of pkg. */
    fun nativeShareAppNotification(pkg: String, app: String, title: String, body: String) =
        call { it.shareAppNotification(pkg, app, title, body) }
}
