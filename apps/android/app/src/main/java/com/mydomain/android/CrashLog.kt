package com.mydomain.android

import android.content.Context
import android.os.Build
import android.util.Log
import java.io.File
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale

/**
 * The local crash log: what took the app down, kept in the app's private
 * files (never sent anywhere), to read and share from Settings (and the
 * Developer screen in debug builds).
 *
 * - Kotlin: an uncaught exception on any thread is written to [FILE] by
 *   [install]'s handler, then Android's own handler runs (the app closes
 *   as before).
 * - Rust: the bridge's panic hook appends each panic to [RUST_FILE]
 *   (`mydomain_net`'s `PANIC_LOG`), beside the session file.
 *
 * Each file keeps its newest [MAX_BYTES]. No crash: no file.
 */
object CrashLog {
    const val FILE = "crash.log"
    /** Must match `PANIC_LOG` in apps/android/rust/src/lib.rs. */
    const val RUST_FILE = "rust-panics.log"
    private const val MAX_BYTES = 64 * 1024
    private const val TAG = "mydomain_crash"

    @Volatile private var dir: File? = null

    /** Catch uncaught exceptions from now on (MdApp.onCreate, once). */
    fun install(ctx: Context) {
        val d = ctx.applicationContext.filesDir
        dir = d
        val previous = Thread.getDefaultUncaughtExceptionHandler()
        Thread.setDefaultUncaughtExceptionHandler { thread, e ->
            runCatching { write(File(d, FILE), entry(thread.name, e)) }
            previous?.uncaughtException(thread, e)
        }
    }

    /** One crash: when, the app and Android versions, the thread, the trace. */
    fun entry(thread: String, e: Throwable, now: Date = Date()): String {
        val ts = SimpleDateFormat("yyyy-MM-dd HH:mm:ss Z", Locale.US).format(now)
        return "--- $ts crash on thread '$thread' (app ${BuildConfig.VERSION_NAME}, Android ${Build.VERSION.RELEASE}, ${Build.MODEL})\n" +
            e.stackTraceToString().trimEnd() + "\n"
    }

    private fun write(f: File, entry: String) {
        val old = if (f.exists()) f.readText() else ""
        var text = old + entry
        if (text.length > MAX_BYTES) {
            // The oldest crashes go, whole.
            val cut = text.indexOf("\n--- ", text.length - MAX_BYTES / 2)
            text = if (cut >= 0) text.substring(cut + 1) else text.takeLast(MAX_BYTES / 2)
        }
        f.writeText(text)
    }

    /** Everything kept (the app's crashes, then the core's panics); empty
     *  when there were none. */
    fun read(ctx: Context): String {
        val d = dir ?: ctx.applicationContext.filesDir
        val parts = listOf(FILE to "App", RUST_FILE to "Core (Rust)").mapNotNull { (name, label) ->
            val f = File(d, name)
            val t = runCatching { if (f.exists()) f.readText().trim() else "" }.getOrDefault("")
            if (t.isEmpty()) null else "== $label ==\n$t"
        }
        return parts.joinToString("\n\n")
    }

    fun isEmpty(ctx: Context): Boolean = read(ctx).isEmpty()

    /** Forget every crash kept. */
    fun clear(ctx: Context) {
        val d = dir ?: ctx.applicationContext.filesDir
        listOf(FILE, RUST_FILE).forEach { runCatching { File(d, it).delete() } }
        Log.i(TAG, "crash log cleared")
    }
}
