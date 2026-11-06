package com.mydomain.android

import android.app.Activity
import android.content.Intent
import android.os.Build
import android.os.Bundle
import android.widget.Toast
import com.mydomain.android.core.Core
import com.mydomain.android.core.ReadLater
import com.mydomain.android.core.ReadLaterLinks
import com.mydomain.android.core.friendlyError
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/**
 * "Read later" in the share sheet, next to my-domain's own share target: a
 * shared link (a browser's page, a link in a message) is saved to the Read
 * later list, which every device of yours has (ReadLater). The title is
 * the one the share gave (its subject, or the words around the link);
 * nothing is fetched from the web. No window: it's translucent, says how
 * it went and closes.
 */
class ReadLaterShareActivity : Activity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        overridePendingTransition(0, 0)
        // Recreated (rotation): the first one is already saving.
        if (savedInstanceState != null) {
            close(null)
            return
        }
        val shared = intent?.takeIf { it.action == Intent.ACTION_SEND }?.let {
            ReadLaterLinks.fromShare(
                it.getCharSequenceExtra(Intent.EXTRA_TEXT)?.toString(),
                it.getStringExtra(Intent.EXTRA_SUBJECT) ?: it.getStringExtra(Intent.EXTRA_TITLE),
            )
        }
        if (shared == null) {
            close(getString(R.string.read_later_share_no_link))
            return
        }
        Core.init(this)
        // The service owns the engine; start it if the app wasn't running.
        val service = Intent(this, NetService::class.java)
        runCatching { if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) startForegroundService(service) else startService(service) }

        scope.launch {
            val msg = if (!awaitEngine()) {
                getString(R.string.read_later_share_failed)
            } else {
                runCatching { withContext(Dispatchers.IO) { ReadLater.saveNow(shared.url, shared.title) } }.fold(
                    { getString(R.string.read_later_saved, it.title) },
                    { friendlyError(it) },
                )
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

    private fun close(msg: String?) {
        if (msg != null) Toast.makeText(applicationContext, msg, Toast.LENGTH_SHORT).show()
        if (!isFinishing) finish()
        overridePendingTransition(0, 0)
    }

    companion object {
        /** Outlives a rotation; the work is short. */
        private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main)
    }
}
