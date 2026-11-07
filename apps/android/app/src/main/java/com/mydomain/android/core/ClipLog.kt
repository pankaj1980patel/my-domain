package com.mydomain.android.core

import android.content.Context
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.os.Handler
import android.os.Looper
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import java.io.File
import java.util.UUID
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
 * Copied pictures sent or received are kept too: the PNG in
 * `files/clip-images/<key>.png` and a JPEG thumbnail (at most 256 px a side)
 * in `<key>.thumb.jpg`; the newest 50, no more than 100 MB in all. A
 * picture's files go when its clip does (pushed out, cleared, history off).
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
    /** The kept pictures' folder, in the app's files dir. */
    const val PICTURE_DIR = "clip-images"
    private const val SAVE_DELAY_MS = 3_000L

    private var all by mutableStateOf<List<ClipRecord>>(emptyList())

    /** The clips, newest first (changed on the main thread); none in sample
     *  mode (debug screenshots), never the real ones then. */
    val list: List<ClipRecord> get() = if (Core.sample) emptyList() else all

    /** "Keep a clipboard history". */
    var enabled by mutableStateOf(true); private set

    private val book = ClipBook()
    private val main = Handler(Looper.getMainLooper())
    private val io = Executors.newSingleThreadExecutor()
    private var file: File? = null
    private var pictureDir: File? = null
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
        val pics = File(app.filesDir, PICTURE_DIR)
        pictureDir = pics
        val on = prefs(app).getBoolean(PREF, true)
        onMain { enabled = on }
        // Pictures kept from before this start, not in the list read back
        // (or all, when it's off), go. Newer files are pictures on their way in.
        val started = System.currentTimeMillis()
        if (!on) {
            io.execute { sweep(pics, emptySet(), started) }
            return
        }
        io.execute {
            val saved = runCatching { if (f.exists()) ClipBook.decode(f.readText()) else emptyList() }.getOrDefault(emptyList())
            onMain {
                // Turned off meanwhile: those are forgotten.
                if (!enabled) return@onMain
                if (saved.isNotEmpty()) {
                    val newer = book.list.isNotEmpty()
                    book.addOlder(saved)
                    // Clips kept while it was reading may have been written
                    // without the older ones: write them all again.
                    if (newer) changed() else {
                        all = book.list
                        forgetDropped()
                    }
                }
                val keep = book.list.mapNotNullTo(HashSet()) { it.picture?.file }
                io.execute { sweep(pics, keep, started) }
            }
        }
    }

    /** Delete the files in [dir] from before [before] that aren't [keep]'s. */
    private fun sweep(dir: File, keep: Set<String>, before: Long) {
        dir.listFiles()?.forEach { f ->
            val key = f.name.substringBefore('.')
            if (key !in keep && f.lastModified() < before) runCatching { f.delete() }
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

    /**
     * [from] sent the picture [png] to this phone's clipboard. Copies it now
     * (before the caller moves it away), then makes its thumbnail off this
     * thread. Not on the main thread.
     */
    fun pictureReceived(ctx: Context, from: String, png: File) {
        val key = copyIn(ctx, png) ?: return
        val at = System.currentTimeMillis()
        io.execute { finishPicture(key, from, true, at) }
    }

    /** This phone sent the picture [png] to [to]'s clipboard. Safe from any thread. */
    fun pictureSent(ctx: Context, to: String, png: File) {
        val at = System.currentTimeMillis()
        io.execute {
            val key = copyIn(ctx, png) ?: return@execute
            finishPicture(key, to, false, at)
        }
    }

    /** A copy of [png] in the pictures' folder, under a new key; none when
     *  the history is off or it can't be copied. */
    private fun copyIn(ctx: Context, png: File): String? {
        if (!enabled) return null
        val dir = pictureDir ?: File(ctx.applicationContext.filesDir, PICTURE_DIR).also { pictureDir = it }
        val key = UUID.randomUUID().toString().replace("-", "")
        return runCatching {
            dir.mkdirs()
            png.copyTo(File(dir, "$key.png"), overwrite = true)
            key
        }.getOrNull()
    }

    /** Its size and thumbnail (on [io]), then kept (main thread). */
    private fun finishPicture(key: String, node: String, incoming: Boolean, at: Long) {
        val dir = pictureDir ?: return
        val png = File(dir, "$key.png")
        val picture = runCatching { makeThumb(dir, key, png) }.getOrNull()
        if (picture == null) {
            deleteFiles(key)
            return
        }
        onMain {
            if (!enabled || book.addPicture(node, incoming, picture, at) == null) {
                io.execute { deleteFiles(key) }
                return@onMain
            }
            changed()
        }
    }

    /** [png]'s size, with its thumbnail written beside it. */
    private fun makeThumb(dir: File, key: String, png: File): ClipPicture? {
        val bounds = BitmapFactory.Options().apply { inJustDecodeBounds = true }
        BitmapFactory.decodeFile(png.path, bounds)
        val (w, h) = bounds.outWidth to bounds.outHeight
        if (w <= 0 || h <= 0) return null
        val opts = BitmapFactory.Options().apply { inSampleSize = ClipBook.sampleSize(w, h, ClipBook.THUMB_MAX) }
        val big = BitmapFactory.decodeFile(png.path, opts) ?: return null
        val (tw, th) = ClipBook.thumbSize(big.width, big.height)
        val small = Bitmap.createScaledBitmap(big, tw, th, true)
        File(dir, "$key.thumb.jpg").outputStream().use { small.compress(Bitmap.CompressFormat.JPEG, 85, it) }
        if (small !== big) small.recycle()
        big.recycle()
        return ClipPicture(key, w, h, png.length())
    }

    /** Kept picture [p]'s PNG. */
    fun pictureFile(p: ClipPicture): File? = pictureDir?.let { File(it, "${p.file}.png") }

    /** Kept picture [p]'s thumbnail. */
    fun thumbFile(p: ClipPicture): File? = pictureDir?.let { File(it, "${p.file}.thumb.jpg") }

    private fun deleteFiles(key: String) {
        val dir = pictureDir ?: return
        runCatching { File(dir, "$key.png").delete() }
        runCatching { File(dir, "$key.thumb.jpg").delete() }
    }

    /** Delete the files of pictures that fell out of the list. */
    private fun forgetDropped() {
        val gone = book.takeDropped()
        if (gone.isNotEmpty()) io.execute { gone.forEach { deleteFiles(it.file) } }
    }

    /** [node]'s clips matching [query]: the first [limit]. */
    fun page(node: String, query: String, limit: Int): ClipPage = ClipBook.page(list, node, query, limit)

    /** Forget [node]'s clips (main thread). */
    fun clear(node: String) {
        if (Core.sample) return
        if (book.clear(node)) changed()
    }

    /** Turn the history on or off; off also forgets everything kept, on
     *  this phone's storage too, now (main thread). */
    fun setEnabled(ctx: Context, on: Boolean) {
        if (Core.sample) return
        enabled = on
        prefs(ctx).edit().putBoolean(PREF, on).apply()
        if (!on) {
            book.clearAll()
            book.takeDropped()
            all = book.list
            val f = file ?: return
            val pics = pictureDir
            io.execute {
                runCatching { f.delete() }
                pics?.listFiles()?.forEach { runCatching { it.delete() } }
            }
        }
    }

    private fun prefs(ctx: Context) = ctx.getSharedPreferences("md", Context.MODE_PRIVATE)

    private fun changed() {
        all = book.list
        forgetDropped()
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
