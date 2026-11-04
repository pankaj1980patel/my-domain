package com.mydomain.android.core

import android.content.Context
import android.os.Handler
import android.os.Looper
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import java.io.File
import java.util.concurrent.Executors

/**
 * Clipboard history: the text this phone and your other devices sent each
 * other as clipboard (`send_text` with mode "clipboard", either way), kept so
 * it can be copied or sent again later. Like the desktop's
 * `clip_history.rs` (the rules are in [ClipBook]):
 *
 *  - the last 200 clips (all devices together), newest first, in
 *    `clipboard-history.json` in the app's files dir, written off the main
 *    thread a few seconds after a change rather than on every clip;
 *  - a sent clip is kept once the core took it (sent, or "queued");
 *  - "Clear" on a device's screen forgets that device's clips.
 *
 * "Keep a clipboard history" in Settings turns it off (which also forgets
 * everything kept); on by default. In the "md" SharedPreferences, key
 * [PREF] (a boolean).
 *
 * This is separate from [Core.clipHistory], the last few clipboard values
 * kept in memory until the app closes.
 */
object ClipLog {
    const val PREF = "clip_history"
    private const val FILE = "clipboard-history.json"
    private const val SAVE_DELAY_MS = 3_000L

    /** The clips, newest first (changed on the main thread). */
    var list by mutableStateOf<List<ClipRecord>>(emptyList()); private set

    /** "Keep a clipboard history". */
    var enabled by mutableStateOf(true); private set

    private val book = ClipBook()
    private val main = Handler(Looper.getMainLooper())
    private val io = Executors.newSingleThreadExecutor()
    private var file: File? = null
    private var savePending = false

    private fun onMain(block: () -> Unit) {
        if (Looper.myLooper() == Looper.getMainLooper()) block() else main.post(block)
    }

    /** Read the setting, and the saved clips off the main thread, once
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
            val saved = runCatching { if (f.exists()) ClipBook.decode(f.readText()) else emptyList() }.getOrDefault(emptyList())
            if (saved.isEmpty()) return@execute
            onMain {
                // Turned off meanwhile: those are forgotten.
                if (!enabled) return@onMain
                val newer = book.list.isNotEmpty()
                book.addOlder(saved)
                // Clips kept while it was reading may have been written
                // without the older ones: write them all again.
                if (newer) {
                    changed()
                } else {
                    list = book.list
                }
            }
        }
    }

    /** [from] sent [text] to this phone's clipboard. Safe from any thread. */
    fun received(from: String, text: String) = add(from, true, text)

    /** This phone sent [text] to [to]'s clipboard. Safe from any thread. */
    fun sent(to: String, text: String) = add(to, false, text)

    private fun add(node: String, incoming: Boolean, text: String) {
        val at = System.currentTimeMillis()
        onMain {
            if (!enabled) return@onMain
            if (book.add(node, incoming, text, at) != null) changed()
        }
    }

    /** [node]'s clips matching [query]: the first [limit]. */
    fun page(node: String, query: String, limit: Int): ClipPage = ClipBook.page(list, node, query, limit)

    /** Forget [node]'s clips (main thread). */
    fun clear(node: String) {
        if (book.clear(node)) changed()
    }

    /** Turn the history on or off; off also forgets everything kept, on
     *  this phone's storage too, now (main thread). */
    fun setEnabled(ctx: Context, on: Boolean) {
        enabled = on
        prefs(ctx).edit().putBoolean(PREF, on).apply()
        if (!on) {
            book.clearAll()
            list = book.list
            val f = file ?: return
            io.execute { runCatching { f.delete() } }
        }
    }

    private fun prefs(ctx: Context) = ctx.getSharedPreferences("md", Context.MODE_PRIVATE)

    private fun changed() {
        list = book.list
        scheduleSave()
    }

    /** Write the file a few seconds from now (once, however many clips come). */
    private fun scheduleSave() {
        if (savePending) return
        savePending = true
        main.postDelayed({
            savePending = false
            val f = file ?: return@postDelayed
            if (!enabled) return@postDelayed
            val snapshot = book.list
            io.execute {
                // Write beside it, then swap it in, so a crash can't leave half a file.
                runCatching {
                    val json = ClipBook.encode(snapshot)
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
