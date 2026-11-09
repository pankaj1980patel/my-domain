package com.mydomain.android.core

import android.content.Context
import android.os.Handler
import android.os.Looper
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import java.io.File
import java.util.concurrent.Executors

/**
 * Message history: the chats between this phone and each of your devices,
 * kept so they're still there after the app restarts. Like the desktop's
 * `chat_history.rs` (the rules are in [ChatLog]):
 *
 *  - per device the newest 2,000 messages, 20,000 over all devices, each up
 *    to 64 KB, in `chat-history.json` in the app's files dir, read at
 *    [Core.init] and written off the main thread a few seconds after a change
 *    rather than on every message;
 *  - recorded where messages already pass in [Core]: `message_received`,
 *    `message_delivered`, and [Core.sendMessage] (queued, sent or failed);
 *  - the chat shows the newest 100 and steps back 100 at a time;
 *  - "Clear chat" on a device's screen forgets that device's messages, on
 *    this phone's storage too.
 *
 * "Keep message history" in Settings turns it off (which also forgets
 * everything kept, and stops saving); on by default. In the "md"
 * SharedPreferences, key [PREF] (a boolean).
 *
 * Local only: never synced to your other devices or the server.
 */
object ChatHistory {
    const val PREF = "chat_history"
    private const val FILE = "chat-history.json"
    private const val SAVE_DELAY_MS = 3_000L

    /** "Keep message history". */
    var enabled by mutableStateOf(true); private set

    /** Bumped on every change, so the chat redraws (main thread). */
    var version by mutableIntStateOf(0); private set

    private var log = ChatLog()
    private val main = Handler(Looper.getMainLooper())
    private val io = Executors.newSingleThreadExecutor()
    private var file: File? = null
    private var savePending = false

    private fun onMain(block: () -> Unit) {
        if (Looper.myLooper() == Looper.getMainLooper()) block() else main.post(block)
    }

    /** Read the setting, and the saved messages off the main thread, once
     *  (from [Core.init]). */
    @Synchronized
    fun load(ctx: Context) {
        if (file != null) return
        val app = ctx.applicationContext
        val f = File(app.filesDir, FILE)
        file = f
        val on = prefs(app).getBoolean(PREF, true)
        onMain { enabled = on }
        if (!on) return
        io.execute {
            val saved = runCatching { HistoryFile.read(f, HistoryFile::isJsonList, Core::historyLog)?.let { ChatLog.read(it) } }.getOrNull()
            if (saved == null || saved.total == 0) return@execute
            onMain {
                // Turned off meanwhile: those are forgotten.
                if (!enabled) return@onMain
                if (log.total == 0) {
                    log = saved
                    version++
                } else {
                    // Messages kept while it was reading: write them all again.
                    log.addOlder(saved.snapshot())
                    changed()
                }
            }
        }
    }

    /** [from] sent [text] here ([id]: the sender's, when it gave one). Safe
     *  from any thread. */
    fun received(from: String, id: String?, text: String) {
        val at = System.currentTimeMillis()
        val msgId = id?.takeIf { it.isNotEmpty() } ?: java.util.UUID.randomUUID().toString()
        record(ChatEntry(msgId, from, null, text, false, at, ChatStatus.Received))
    }

    /** This phone is sending [text] to [to] as message [id]: kept as queued
     *  until [sendResult]. Safe from any thread. */
    fun sending(to: String, id: String, text: String) {
        record(ChatEntry(id, null, to, text, false, System.currentTimeMillis(), ChatStatus.Queued))
    }

    /** How sending message [id] went. A receipt that came first isn't
     *  undone. Safe from any thread. */
    fun sendResult(id: String, status: ChatStatus) = update(id, status)

    /** The device confirmed message [id] arrived. Safe from any thread. */
    fun delivered(id: String) = update(id, ChatStatus.Delivered)

    private fun record(e: ChatEntry) = onMain {
        if (enabled && log.add(e)) changed()
    }

    private fun update(id: String, status: ChatStatus) = onMain {
        if (enabled && log.setStatus(id, status)) changed()
    }

    /** [node]'s messages before message [before] (the newest without it):
     *  the last [limit], oldest first (main thread). Read from a composable,
     *  it redraws on every change ([version]). */
    fun page(node: String, before: String?, limit: Int): ChatPage =
        version.let { if (Core.sample) ChatPage(emptyList(), more = false, kept = 0) else log.page(node, before, limit) }

    /** A copy of every kept message, oldest first per device (main thread).
     *  Read only: for a search over this phone's messages; read from a
     *  composable, it redraws on every change. */
    fun snapshot(): List<ChatEntry> = version.let { if (Core.sample) emptyList() else log.snapshot() }

    /** Forget [node]'s messages (main thread). */
    fun clear(node: String) {
        if (Core.sample) return
        if (log.clear(node)) changed()
    }

    /** Turn the history on or off; off also forgets everything kept, on
     *  this phone's storage too, now (main thread). */
    fun setEnabled(ctx: Context, on: Boolean) {
        if (Core.sample) return
        enabled = on
        prefs(ctx).edit().putBoolean(PREF, on).apply()
        if (!on) {
            log.clearAll()
            version++
            val f = file ?: return
            io.execute { runCatching { f.delete() } }
        }
    }

    /** Forget every message, on this phone's storage too, keeping the
     *  setting (signing out: the next account mustn't see them). Main thread. */
    fun wipe() {
        log.clearAll()
        version++
        val f = file ?: return
        // After any save already queued (same thread), so nothing comes back.
        io.execute { runCatching { f.delete() } }
    }

    private fun prefs(ctx: Context) = ctx.getSharedPreferences("md", Context.MODE_PRIVATE)

    private fun changed() {
        version++
        scheduleSave()
    }

    /** Write the file a few seconds from now (once, however many messages
     *  come). */
    private fun scheduleSave() {
        if (savePending) return
        savePending = true
        main.postDelayed({
            savePending = false
            val f = file ?: return@postDelayed
            if (!enabled) return@postDelayed
            val snapshot = log.snapshot()
            io.execute {
                // Write beside it, then swap it in, so a crash can't leave half a file.
                runCatching {
                    val json = ChatLog.encode(snapshot)
                    val tmp = File(f.path + ".tmp")
                    tmp.writeText(json)
                    if (!tmp.renameTo(f)) {
                        f.writeText(json)
                        tmp.delete()
                    }
                }
            }
        }, SAVE_DELAY_MS)
    }
}
