package com.mydomain.android.core

import android.content.Context
import android.os.Handler
import android.os.Looper
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.setValue
import org.json.JSONObject
import java.io.File
import java.util.concurrent.Executors

/**
 * The Activity timeline's own record (rules in [ActivityRecord]): what no
 * history on the phone keeps — devices connecting and going away, calls,
 * rings, links opened, screens, cameras and sounds shared, key changes,
 * wake-ups — in `activity.json` in the app's files dir, written off the main
 * thread a few seconds after a change. The kinds hidden and "Clear activity"
 * are kept there too. The timeline itself ([page]) merges this with the
 * transfers, clipboard and message histories, notes, saved links and
 * timers ([ActivityFeed]). Local only.
 */
object ActivityLog {
    private const val FILE = "activity.json"
    private const val SAVE_DELAY_MS = 3_000L

    /** Bumped on every change (main thread), so the screen redraws. */
    var version by mutableIntStateOf(0); private set

    private var rec = ActivityRecord()
    private val main = Handler(Looper.getMainLooper())
    private val io = Executors.newSingleThreadExecutor()
    private var file: File? = null
    private var savePending = false

    private fun onMain(block: () -> Unit) {
        if (Looper.myLooper() == Looper.getMainLooper()) block() else main.post(block)
    }

    /** Read the saved record off the main thread, once (from [Core.init]). */
    @Synchronized
    fun load(ctx: Context) {
        if (file != null) return
        val f = File(ctx.applicationContext.filesDir, FILE)
        file = f
        io.execute {
            val saved = runCatching { if (f.exists()) ActivityRecord.decode(f.readText()) else null }.getOrNull() ?: return@execute
            onMain {
                // Entries kept while it was reading go on top.
                val fresh = rec.entries
                rec = saved
                fresh.reversed().forEach { rec.push(it.action, it.node, it.subject, it.detail, it.ts) }
                version++
            }
        }
    }

    /** Note something (see [ActivityRow] for the actions). Safe from any thread. */
    fun record(action: String, node: String?, subject: String = "", detail: String = "") {
        val now = System.currentTimeMillis()
        onMain { if (rec.push(action, node?.ifEmpty { null }, subject, detail, now)) changed() }
    }

    /** What the timeline keeps of a core event (the rest is in the histories). Main thread. */
    fun onEvent(o: JSONObject) {
        val node = o.optString("node_id")
        val from = o.optString("from")
        val onOff = if (o.optBoolean("on")) "on" else "off"
        when (o.optString("kind")) {
            "peer_connected" -> record("link_up", node, detail = o.optString("transport"))
            "peer_disconnected" -> record("link_down", node)
            "call_notification" -> record("call", from, o.optString("caller"), o.optString("state"))
            "ring" -> record("ring", from)
            "open_url" -> record("open_url", from, o.optString("url"))
            "key_changed" -> record("key_changed", node, o.optString("name"))
            "screen_watch" -> record("screen_watch", from, detail = onOff)
            "screen_offer" -> record("screen_offer", from, detail = onOff)
            "camera_watch" -> record("camera_watch", from, detail = onOff)
            "audio_offer" -> record("audio_offer", from, detail = onOff)
            "audio_listen" -> record("audio_listen", from, detail = onOff)
        }
    }

    /** The record's entries, newest first (main thread; read from a composable it redraws). */
    fun entries(): List<ActivityEntry> = version.let { rec.entries }

    /** The kinds hidden. */
    fun hidden(): List<String> = version.let { rec.hidden }

    /** Since when the view was cleared, if it was. */
    fun clearedAt(): Long? = version.let { rec.clearedAt }

    fun setHidden(kinds: Collection<String>) {
        rec.setHidden(kinds)
        changed()
    }

    /** "Clear activity": only this record goes; the histories stay. */
    fun clear() {
        rec.clear(System.currentTimeMillis())
        changed()
    }

    fun restore() {
        rec.restore()
        changed()
    }

    /** One page of the timeline from what the phone keeps now (main thread). */
    fun page(q: ActivityQuery): ActivityPage {
        val known = Core.peers.map { it.nodeId }.toSet()
        val src = ActivitySources(
            names = known.associateWith { Core.peerName(it) },
            transfers = TransferLog.list,
            clips = ClipLog.list,
            messages = ChatHistory.snapshot(),
            notes = Notes.byId.values.map { n ->
                NoteSource(n.id, n.title, n.body, n.updatedAt, n.by.takeIf { it in known }, n.conflictOf != null)
            },
            links = ReadLater.byId.values.map(ReadLater::item),
            timers = Timers.byId.values.map { TimerSource(it.id, it.label, it.durationMs, it.createdAt, it.by.takeIf { b -> b in known }) },
            log = rec.entries,
        )
        return ActivityFeed.page(src, q, rec.hidden, rec.clearedAt)
    }

    private fun changed() {
        version++
        if (savePending) return
        savePending = true
        main.postDelayed({
            savePending = false
            val f = file ?: return@postDelayed
            val json = rec.encode()
            io.execute {
                runCatching {
                    val tmp = File(f.path + ".tmp")
                    tmp.writeText(json)
                    if (!tmp.renameTo(f)) {
                        f.delete()
                        tmp.renameTo(f)
                    }
                }
            }
        }, SAVE_DELAY_MS)
    }
}
