package com.mydomain.android.core

import android.util.Log
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineExceptionHandler
import kotlinx.coroutines.CoroutineName
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.launch

/**
 * The app's one scope for background work that isn't tied to a screen or a
 * service (it lives as long as the process). It replaced the bare
 * `Thread { }.start()` calls: one failing job is logged and doesn't take
 * the others (or the app) with it, which an exception on a bare thread did.
 *
 * Supervised, on [Dispatchers.Default]; [bg] runs its block on
 * [Dispatchers.IO] (same threads, more of them) because most of this work
 * waits on the Rust core or the disk.
 */
object AppScope {
    private const val TAG = "mydomain_bg"

    private val failed = CoroutineExceptionHandler { ctx, e ->
        Log.e(TAG, "background work ${ctx[CoroutineName]?.name ?: ""} failed", e)
    }

    val scope = CoroutineScope(SupervisorJob() + Dispatchers.Default + failed)

    /** Run [block] in the background, in place of `Thread { }.start()`:
     *  [name] says what it is in the log if it fails. */
    fun bg(name: String, block: suspend CoroutineScope.() -> Unit): Job =
        scope.launch(Dispatchers.IO + CoroutineName(name), block = block)
}

/**
 * Call into the Rust core where a failure should be logged and shrugged
 * off: the result, or null when it threw (the core's `MdException`, a Rust
 * panic coming back as an exception, or the library not loading). [what]
 * names the call in the log. Coroutine cancellation still goes through.
 *
 * Use it instead of a bare `runCatching { Core.engine()?.x() }`, which
 * hides why a call failed.
 */
inline fun <T> ffi(what: String, block: () -> T): T? = try {
    block()
} catch (e: CancellationException) {
    throw e
} catch (e: Exception) {
    ffiFailed(what, e)
    null
} catch (e: LinkageError) {
    // The native library is missing or doesn't match its bindings.
    ffiFailed(what, e)
    null
}

@PublishedApi
internal fun ffiFailed(what: String, e: Throwable) {
    runCatching { Log.w("mydomain_ffi", "$what failed: $e") }
}
