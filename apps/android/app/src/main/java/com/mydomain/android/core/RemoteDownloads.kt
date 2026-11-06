package com.mydomain.android.core

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateMapOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import com.mydomain.android.ffi.RemoteDownloadEntry
import com.mydomain.android.ffi.RemoteDownloadPrefs
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import org.json.JSONObject

/**
 * "Download on <device>" (the core's `features::remote_download`): every
 * download either way by id, from the core's `remote_download` events
 * ([onEvent], main thread). Its progress and end come as the usual file_*
 * events with the same id, so it shows (and is cancelled) as a transfer;
 * what a transfer can't say (the link's host, asking, queued) is here.
 *
 * The rules (rows, which devices to offer, size limits) are
 * [RemoteDownloadPlan]; the ask notification is
 * [com.mydomain.android.DownloadAsk].
 */
object RemoteDownloads {
    val byId = mutableStateMapOf<String, DlItem>()

    /** A link waiting for "Download on…" (the device chooser shows). */
    var pendingLink by mutableStateOf<String?>(null)

    /** Questions waiting on this phone, the oldest first. */
    val asks: List<DlItem> get() = RemoteDownloadPlan.asks(byId.values)

    fun parse(o: JSONObject): DlItem? {
        val id = o.optString("id").ifEmpty { return null }
        return DlItem(
            id = id,
            node = o.optString("node_id"),
            here = o.optBoolean("here"),
            url = o.optString("url"),
            host = o.optString("host"),
            name = o.optString("name"),
            size = o.optLong("size"),
            done = o.optLong("done"),
            state = o.optString("state"),
            error = if (o.isNull("error")) null else o.optString("error").ifEmpty { null },
            startedAt = o.optLong("started_at"),
        )
    }

    private fun of(e: RemoteDownloadEntry) = DlItem(
        e.id, e.nodeId, e.here, e.url, e.host, e.name, e.size, e.done, e.state, e.error, e.startedAt,
    )

    /** A `remote_download` event (`{"item": {...}}`): the item. Main thread. */
    fun onEvent(o: JSONObject): DlItem? {
        val item = o.optJSONObject("item")?.let(::parse) ?: return null
        byId[item.id] = item
        TransferLog.link(item.id, item.node, item.name, item.here, item.host)
        if (byId.size > 100) {
            byId.values.filter { it.finished }.sortedBy { it.startedAt }
                .take(byId.size - 100).forEach { byId.remove(it.id) }
        }
        return item
    }

    /** Read what the core has (after a restart of the screen). Safe from
     *  any thread. */
    fun refresh() {
        val list = runCatching { Core.engine()?.remoteDownloads() }.getOrNull() ?: return
        android.os.Handler(android.os.Looper.getMainLooper()).post {
            list.forEach { byId[it.id] = of(it) }
        }
    }

    /** Whether [node] downloads links. Blocking (a quick read). */
    fun supported(node: String): Boolean =
        runCatching { Core.engine()?.remoteDownloadSupported(node) }.getOrNull() == true

    /** Ask [node] to download [link]. */
    suspend fun request(node: String, link: String): DlItem =
        of(Core.call { it.remoteDownloadRequest(node, link) })

    /** Answer the question about [id]. */
    suspend fun answer(id: String, allow: Boolean, always: Boolean) =
        Core.call { it.remoteDownloadAnswer(id, allow, always) }

    suspend fun settings(): RemoteDownloadPrefs = Core.call { it.remoteDownloadSettings() }

    suspend fun setSettings(p: RemoteDownloadPrefs) = withContext(Dispatchers.IO) {
        Core.call { it.setRemoteDownloadSettings(p) }
    }
}
