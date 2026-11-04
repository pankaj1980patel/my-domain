package com.mydomain.android.core

enum class TransferState { Active, Done, Failed }

/**
 * One transfer in the Transfers list, either way. [uri]/[mime]: a received
 * file's copy in Downloads (to open it). [source]: the content Uri a sent
 * file was picked from (to send it again). [resumedFrom]: it carried on
 * from a partial file at that many bytes. [rate] (bytes a second, smoothed)
 * and [movedAt] (when [done] last moved) are for speed and time left, and
 * aren't saved.
 */
data class TransferRecord(
    val id: String,
    val node: String,
    val name: String,
    val incoming: Boolean,
    val state: TransferState = TransferState.Active,
    val done: Long = 0,
    val total: Long = 0,
    val resumedFrom: Long? = null,
    val reason: String? = null,
    val uri: String? = null,
    val mime: String? = null,
    val source: String? = null,
    val started: Long = 0,
    val ended: Long? = null,
    val rate: Double = 0.0,
    val movedAt: Long = 0,
) {
    val finished: Boolean get() = state != TransferState.Active
    val fraction: Float get() = if (total > 0) (done.toFloat() / total).coerceIn(0f, 1f) else 0f

    /** Seconds left at the current speed; null when not known. */
    fun secondsLeft(): Long? =
        if (rate > 0 && total > done) Math.round((total - done) / rate) else null
}

/**
 * The Transfers list's rules (plain Kotlin, so JVM tests cover them; the
 * phone-side file and threads are in [TransferLog]). Like the desktop's
 * `transfers.rs`:
 *
 *  - one entry per transfer id, updated in place by each event;
 *  - those under way are listed first (newest first), then the finished and
 *    failed ones (latest ended first), the newest [MAX_FINISHED] of them;
 *  - only finished ones are saved (one under way comes back through its
 *    events after a restart).
 */
class TransferBook(saved: List<TransferRecord> = emptyList()) {
    private val items = ArrayList<TransferRecord>()

    init {
        addSaved(saved)
    }

    /** Saved entries read after newer ones arrived: those not here yet. */
    fun addSaved(saved: List<TransferRecord>) {
        val have = items.map { it.id }.toHashSet()
        saved.filter { it.finished && it.id !in have }.forEach { items.add(it) }
        trim()
    }

    fun get(id: String): TransferRecord? = items.firstOrNull { it.id == id }

    private inline fun upsert(id: String, create: () -> TransferRecord, change: (TransferRecord) -> TransferRecord?): Boolean {
        if (id.isEmpty()) return false
        val i = items.indexOfFirst { it.id == id }
        val cur = if (i >= 0) items[i] else create()
        val next = change(cur) ?: return false
        if (i >= 0) items[i] = next else items.add(next)
        return true
    }

    /** A transfer moved on. Late progress for one that ended is ignored. */
    fun progress(id: String, node: String, name: String, done: Long, total: Long, incoming: Boolean, resumed: Boolean, now: Long) {
        upsert(id, { TransferRecord(id, node, name, incoming, started = now) }) { t ->
            if (t.finished) return@upsert null
            var rate = t.rate
            if (t.movedAt in 1 until now && done > t.done) {
                val inst = (done - t.done) * 1000.0 / (now - t.movedAt)
                rate = if (rate <= 0) inst else rate * 0.7 + inst * 0.3
            }
            t.copy(
                done = done,
                total = total,
                resumedFrom = t.resumedFrom ?: if (resumed) done else null,
                rate = rate,
                movedAt = if (done != t.done || t.movedAt == 0L) now else t.movedAt,
            )
        }
    }

    /** We started sending [name] (picked from [source], [size] bytes). */
    fun sending(id: String, node: String, name: String, source: String?, size: Long, now: Long) {
        upsert(id, { TransferRecord(id, node, name, false, started = now) }) { t ->
            t.copy(source = source, total = if (t.total == 0L) size else t.total)
        }
    }

    /** A file arrived. Whether it changed what's saved. */
    fun received(id: String, from: String, name: String, size: Long, now: Long): Boolean =
        finish(upsert(id, { TransferRecord(id, from, name, true, started = now) }) { t ->
            if (t.state == TransferState.Done) null
            else t.copy(state = TransferState.Done, done = size, total = size, reason = null, ended = now)
        })

    /** The received file [id] is in Downloads as [uri]: it can be opened. */
    fun saved(id: String, uri: String, mime: String?): Boolean =
        upsert(id, { return false }) { it.copy(uri = uri, mime = mime) }

    /** The other device saved our file. */
    fun sent(id: String, node: String, name: String, now: Long): Boolean =
        finish(upsert(id, { TransferRecord(id, node, name, false, started = now) }) { t ->
            if (t.state == TransferState.Done) null
            else t.copy(state = TransferState.Done, done = t.total, reason = null, ended = now)
        })

    /** A transfer stopped for good. One already done stays done. */
    fun failed(id: String, node: String, name: String, incoming: Boolean, reason: String, now: Long): Boolean =
        finish(upsert(id, { TransferRecord(id, node, name, incoming, started = now) }) { t ->
            if (t.state == TransferState.Done) null
            else t.copy(state = TransferState.Failed, reason = reason, ended = now)
        })

    private fun finish(changed: Boolean): Boolean {
        if (changed) trim()
        return changed
    }

    /** Forget every finished transfer. Whether there were any. */
    fun clearFinished(): Boolean = items.removeAll { it.finished }

    private fun trim() {
        val finished = items.filter { it.finished }
        if (finished.size <= MAX_FINISHED) return
        val drop = finished.sortedByDescending { it.ended ?: it.started }.drop(MAX_FINISHED).map { it.id }.toHashSet()
        items.removeAll { it.id in drop }
    }

    /** What the screen shows: under way first (newest first), then the
     *  finished ones (latest ended first). */
    val list: List<TransferRecord>
        get() = items.filter { !it.finished }.sortedByDescending { it.started } +
            items.filter { it.finished }.sortedByDescending { it.ended ?: it.started }

    /** What's saved: the finished ones. */
    fun saved(): List<TransferRecord> = list.filter { it.finished }

    companion object {
        /** The most finished transfers kept. */
        const val MAX_FINISHED = 100

        /** The file's text: `{"transfers": [...]}`, finished ones only. */
        fun encode(list: List<TransferRecord>): String = Json.write(
            mapOf(
                "transfers" to list.filter { it.finished }.map { t ->
                    linkedMapOf(
                        "id" to t.id,
                        "node" to t.node,
                        "name" to t.name,
                        "incoming" to t.incoming,
                        "state" to t.state.name.lowercase(),
                        "done" to t.done,
                        "total" to t.total,
                        "resumed_from" to t.resumedFrom,
                        "reason" to t.reason,
                        "uri" to t.uri,
                        "mime" to t.mime,
                        "source" to t.source,
                        "started" to t.started,
                        "ended" to t.ended,
                    )
                },
            ),
        )

        /** The finished transfers in the file's [text]; unreadable text or
         *  entries are left out. */
        fun decode(text: String): List<TransferRecord> {
            val root = runCatching { Json.parse(text) }.getOrNull() as? Map<*, *> ?: return emptyList()
            val items = root["transfers"] as? List<*> ?: return emptyList()
            return items.mapNotNull { item ->
                val o = item as? Map<*, *> ?: return@mapNotNull null
                val id = o["id"] as? String ?: return@mapNotNull null
                val state = when (o["state"]) {
                    "done" -> TransferState.Done
                    "failed" -> TransferState.Failed
                    else -> return@mapNotNull null
                }
                TransferRecord(
                    id = id,
                    node = o["node"] as? String ?: "",
                    name = o["name"] as? String ?: "",
                    incoming = o["incoming"] as? Boolean ?: false,
                    state = state,
                    done = (o["done"] as? Number)?.toLong() ?: 0,
                    total = (o["total"] as? Number)?.toLong() ?: 0,
                    resumedFrom = (o["resumed_from"] as? Number)?.toLong(),
                    reason = o["reason"] as? String,
                    uri = o["uri"] as? String,
                    mime = o["mime"] as? String,
                    source = o["source"] as? String,
                    started = (o["started"] as? Number)?.toLong() ?: 0,
                    ended = (o["ended"] as? Number)?.toLong(),
                )
            }.distinctBy { it.id }
        }
    }
}
