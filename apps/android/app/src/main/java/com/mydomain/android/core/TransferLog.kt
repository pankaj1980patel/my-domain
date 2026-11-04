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
 * The Transfers screen's list: every file going to or coming from your
 * devices, both ways (the rules are in [TransferBook]). Fed by the core's
 * `file_*` events (via [Core]) and by [Core.sendFile]; the finished ones are
 * kept in `transfer-history.json` in the app's files dir, written off the
 * main thread a few seconds after a change.
 *
 * Separate from [Core.transfers], which a device's Files tile shows for
 * this process only.
 */
object TransferLog {
    private const val FILE = "transfer-history.json"
    private const val SAVE_DELAY_MS = 3_000L

    /** Under way first, then the finished ones (changed on the main thread). */
    var list by mutableStateOf<List<TransferRecord>>(emptyList()); private set

    private val book = TransferBook()
    private val main = Handler(Looper.getMainLooper())
    private val io = Executors.newSingleThreadExecutor()
    private var file: File? = null
    private var savePending = false

    private fun onMain(block: () -> Unit) {
        if (Looper.myLooper() == Looper.getMainLooper()) block() else main.post(block)
    }

    /** Read the saved transfers off the main thread, once (from [Core.init]). */
    @Synchronized
    fun load(ctx: Context) {
        if (file != null) return
        val f = File(ctx.applicationContext.filesDir, FILE)
        file = f
        io.execute {
            val saved = runCatching { if (f.exists()) TransferBook.decode(f.readText()) else emptyList() }.getOrDefault(emptyList())
            if (saved.isEmpty()) return@execute
            onMain {
                val newer = book.saved().isNotEmpty()
                book.addSaved(saved)
                // Finished while it was reading: write them all again.
                if (newer) changed(save = true) else list = book.list
            }
        }
    }

    // --- from the core's events (main thread, via Core.apply) -----------------

    fun progress(id: String, node: String, name: String, done: Long, total: Long, incoming: Boolean, resumed: Boolean) {
        book.progress(id, node, name, done, total, incoming, resumed, System.currentTimeMillis())
        changed(save = false)
    }

    fun received(id: String, from: String, name: String, size: Long) {
        if (book.received(id, from, name, size, System.currentTimeMillis())) changed(save = true)
    }

    fun sent(id: String, node: String, name: String) {
        if (book.sent(id, node, name, System.currentTimeMillis())) changed(save = true)
    }

    fun failed(id: String, node: String, name: String, incoming: Boolean, reason: String) {
        if (book.failed(id, node, name, incoming, reason, System.currentTimeMillis())) changed(save = true)
    }

    /** A received file is in Downloads as [uri]. Safe from any thread. */
    fun saved(id: String, uri: String, mime: String?) = onMain {
        if (book.saved(id, uri, mime)) changed(save = book.get(id)?.finished == true)
    }

    /** [Core.sendFile] handed [name] (picked as [source]) to the core as
     *  transfer [id]. Safe from any thread. */
    fun sending(id: String, node: String, name: String, source: String?, size: Long) = onMain {
        book.sending(id, node, name, source, size, System.currentTimeMillis())
        changed(save = book.get(id)?.finished == true)
    }

    /** Forget the finished and failed transfers (main thread). */
    fun clearFinished() {
        if (book.clearFinished()) changed(save = true)
    }

    private fun changed(save: Boolean) {
        list = book.list
        if (save) scheduleSave()
    }

    /** Write the file a few seconds from now (once, however many change). */
    private fun scheduleSave() {
        if (savePending) return
        savePending = true
        main.postDelayed({
            savePending = false
            val f = file ?: return@postDelayed
            val snapshot = book.saved()
            io.execute {
                // Write beside it, then swap it in, so a crash can't leave half a file.
                runCatching {
                    val json = TransferBook.encode(snapshot)
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
