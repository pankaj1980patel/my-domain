package com.mydomain.android

import android.app.Activity
import android.content.Intent
import android.net.Uri
import android.os.Build
import android.os.Bundle
import android.widget.Toast
import androidx.core.content.pm.ShortcutManagerCompat
import com.mydomain.android.core.Core
import com.mydomain.android.core.DeviceLabels
import com.mydomain.android.core.friendlyError
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch

/**
 * A share sent through one of the share sheet's device targets
 * ([ShareTargets]): MainActivity, the share target, hands the share here
 * (with read access to its files), and this sends it straight to that
 * device, says how it went, and closes. No window: it's translucent.
 *
 * It stays open until the shared files are copied, since read access to
 * them lasts only while an activity that got them is alive.
 */
class DirectShareActivity : Activity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        overridePendingTransition(0, 0)
        // Recreated (rotation): the first one is already sending.
        if (savedInstanceState != null) {
            close(null)
            return
        }
        val node = intent?.getStringExtra(ShortcutManagerCompat.EXTRA_SHORTCUT_ID)
        val uris = sharedUris(intent)
        // Text only with ACTION_SEND, as MainActivity takes it.
        val text = intent?.takeIf { it.action == Intent.ACTION_SEND }?.getCharSequenceExtra(Intent.EXTRA_TEXT)?.toString()
        if (node.isNullOrBlank() || (uris.isEmpty() && text.isNullOrBlank())) {
            close(null)
            return
        }
        Core.init(this)
        // The service owns the engine; start it if the app wasn't running.
        val service = Intent(this, NetService::class.java)
        runCatching { if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) startForegroundService(service) else startService(service) }
        ShareTargets.reportUsed(this, node)

        scope.launch {
            val ready = awaitEngine()
            val name = nameOf(node)
            val msg = when {
                !ready -> getString(R.string.qs_failed, name)
                uris.isNotEmpty() -> {
                    // Said now: copying big files takes a moment. How the
                    // transfer goes shows as for any other file.
                    toast(getString(R.string.share_sending, name))
                    var failed: Throwable? = null
                    for (u in uris) {
                        try { Core.sendFile(node, u) } catch (e: Exception) { if (failed == null) failed = e }
                    }
                    failed?.let(::friendlyError)
                }
                else -> {
                    val shared = text.orEmpty()
                    val link = Core.isSharedLink(shared)
                    runCatching { Core.sendSharedText(node, shared) }.fold(
                        { how ->
                            val queued = how == "queued"
                            when {
                                link && queued -> getString(R.string.find_link_queued, name)
                                link -> getString(R.string.find_link_sent, name)
                                queued -> getString(R.string.qs_clip_queued, name)
                                else -> getString(R.string.qs_clip_sent, name)
                            }
                        },
                        { friendlyError(it) },
                    )
                }
            }
            close(msg)
        }
    }

    /** The core's engine starts with the service, just after a cold start:
     *  wait for it a little. */
    private suspend fun awaitEngine(): Boolean {
        repeat(100) {
            if (Core.engine() != null) return true
            delay(100)
        }
        return Core.engine() != null
    }

    /** Its nickname or name; before the device list has loaded, the name
     *  its shortcut was given. */
    private fun nameOf(node: String): String {
        val known = Core.peers.any { it.nodeId == node } || DeviceLabels.nickname(node) != null
        return if (known) Core.peerName(node) else ShareTargets.labelOf(this, node) ?: Core.peerName(node)
    }

    private fun toast(msg: String) = Toast.makeText(applicationContext, msg, Toast.LENGTH_SHORT).show()

    private fun close(msg: String?) {
        if (msg != null) toast(msg)
        if (!isFinishing) finish()
        overridePendingTransition(0, 0)
    }

    companion object {
        /** Outlives a rotation; the work is short (copying, then the core). */
        private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main)

        /** The files in a share (EXTRA_STREAM). */
        @Suppress("DEPRECATION")
        fun sharedUris(intent: Intent?): List<Uri> {
            intent ?: return emptyList()
            return when (intent.action) {
                Intent.ACTION_SEND -> listOfNotNull(
                    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) intent.getParcelableExtra(Intent.EXTRA_STREAM, Uri::class.java)
                    else intent.getParcelableExtra(Intent.EXTRA_STREAM),
                )
                Intent.ACTION_SEND_MULTIPLE ->
                    (if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) intent.getParcelableArrayListExtra(Intent.EXTRA_STREAM, Uri::class.java)
                    else intent.getParcelableArrayListExtra<Uri>(Intent.EXTRA_STREAM)).orEmpty()
                else -> emptyList()
            }
        }
    }
}
