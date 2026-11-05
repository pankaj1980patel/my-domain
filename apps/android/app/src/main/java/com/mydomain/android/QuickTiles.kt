package com.mydomain.android

import android.app.Activity
import android.content.ClipboardManager
import android.content.Context
import android.content.Intent
import android.os.Bundle
import android.service.quicksettings.TileService
import android.widget.Toast
import com.mydomain.android.core.Core
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/**
 * Quick Settings tiles (the phone's pull-down panel): "Send clipboard to
 * computer" and "Lock computer", one tap from anywhere.
 *
 * "The computer" is the device that takes the `lock` action (computers do,
 * phones don't), preferring one with a live link; the last one is
 * remembered, so the tiles work before any status came in this run.
 */
object QuickTarget {
    private const val PREF = "qs_computer"

    fun computer(ctx: Context): String? {
        val prefs = ctx.getSharedPreferences("md", Context.MODE_PRIVATE)
        val computers = Core.peers.map { it.nodeId }.filter { "lock" in (Core.deviceStatus[it]?.actions ?: emptySet()) }
        val pick = computers.firstOrNull { Core.links.containsKey(it) } ?: computers.firstOrNull()
        if (pick != null) {
            prefs.edit().putString(PREF, pick).apply()
            return pick
        }
        return prefs.getString(PREF, null)?.takeIf { id -> Core.peers.any { it.nodeId == id } }
    }

    fun name(node: String): String = Core.peerName(node)
}

/** Tile: send what you copied to your computer. Android lets only an app
 *  on screen read the clipboard, so this opens [ClipSendActivity] (an
 *  invisible screen that reads it, sends it and closes). */
class ClipTileService : TileService() {
    override fun onClick() {
        val open = QuickActions.clipIntent(this)
        // The app targets Android 9, so the Intent form is allowed.
        @Suppress("DEPRECATION")
        startActivityAndCollapse(open)
    }
}

/** Tile: open the app straight into a new note ([NotesWidget.newNoteIntent];
 *  the widget's "+" and the launcher shortcut do the same). */
class NewNoteTileService : TileService() {
    override fun onClick() {
        val open = NotesWidget.newNoteIntent(this)
        // The app targets Android 9, so the Intent form is allowed.
        @Suppress("DEPRECATION")
        startActivityAndCollapse(open)
    }
}

/** Tile: lock your computer. */
class LockTileService : TileService() {
    override fun onClick() = QuickActions.lock(applicationContext)
}

/** The one-tap actions on the computer, shared by the tiles and the
 *  home-screen widget ([HomeWidget]). Each says how it went in a toast. */
object QuickActions {
    /** Opens [ClipSendActivity], which reads the clipboard and sends it. */
    fun clipIntent(ctx: Context): Intent =
        Intent(ctx, ClipSendActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)

    fun lock(ctx: Context) = onComputer(ctx) { app, node ->
        runCatching { Core.runAction(node, "lock") }
            .fold({ app.getString(R.string.qs_locked, QuickTarget.name(node)) }, { app.getString(R.string.qs_failed, QuickTarget.name(node)) })
    }

    fun ring(ctx: Context) = onComputer(ctx) { app, node ->
        val name = QuickTarget.name(node)
        runCatching { Core.ring(node) }.fold(
            { if (it == "queued") app.getString(R.string.find_ring_queued, name) else app.getString(R.string.find_ringing, name) },
            { app.getString(R.string.qs_failed, name) },
        )
    }

    /** Runs [act] on the computer off the main thread and toasts what it
     *  returns; says so if there's no computer or the app isn't running. */
    private fun onComputer(ctx: Context, act: suspend (Context, String) -> String) {
        val app = ctx.applicationContext
        val node = QuickTarget.computer(app)
        if (node == null || Core.engine() == null) {
            Toast.makeText(app, app.getString(R.string.qs_no_computer), Toast.LENGTH_SHORT).show()
            return
        }
        CoroutineScope(Dispatchers.IO).launch {
            val msg = act(app, node)
            withContext(Dispatchers.Main) { Toast.makeText(app, msg, Toast.LENGTH_SHORT).show() }
        }
    }
}

/** Reads the clipboard once it has focus (Android's rule), sends it to the
 *  computer, says so, and closes. */
class ClipSendActivity : Activity() {
    private var sent = false

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        overridePendingTransition(0, 0)
    }

    override fun onWindowFocusChanged(hasFocus: Boolean) {
        super.onWindowFocusChanged(hasFocus)
        if (!hasFocus || sent) return
        sent = true
        val app = applicationContext
        val node = QuickTarget.computer(app)
        val clip = getSystemService(ClipboardManager::class.java)?.primaryClip
        val text = clip?.takeIf { it.itemCount > 0 }?.getItemAt(0)?.coerceToText(this)?.toString().orEmpty()
        when {
            node == null || Core.engine() == null -> done(getString(R.string.qs_no_computer))
            text.isBlank() -> done(getString(R.string.qs_nothing_copied))
            else -> CoroutineScope(Dispatchers.IO).launch {
                val via = runCatching { Core.sendText(node, text, "clipboard") }
                val name = QuickTarget.name(node)
                val msg = via.fold(
                    { if (it == "queued") getString(R.string.qs_clip_queued, name) else getString(R.string.qs_clip_sent, name) },
                    { getString(R.string.qs_failed, name) },
                )
                withContext(Dispatchers.Main) { done(msg) }
            }
        }
    }

    private fun done(msg: String) {
        Toast.makeText(applicationContext, msg, Toast.LENGTH_SHORT).show()
        finish()
        overridePendingTransition(0, 0)
    }
}
