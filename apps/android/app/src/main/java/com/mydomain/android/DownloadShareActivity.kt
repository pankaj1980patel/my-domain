package com.mydomain.android

import android.app.Activity
import android.content.Intent
import android.os.Bundle
import android.widget.Toast
import com.mydomain.android.core.RemoteDownloadPlan

/**
 * "Download on…" in the share sheet, next to my-domain's own share
 * target and Read later: a shared link (a browser's page, a file's link in
 * a message) is downloaded by another of your devices itself, straight
 * from the web. No window of its own: it hands the link to [MainActivity],
 * which asks which device (only those that download links), and closes.
 */
class DownloadShareActivity : Activity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        overridePendingTransition(0, 0)
        if (savedInstanceState == null) {
            val link = intent?.takeIf { it.action == Intent.ACTION_SEND }?.let {
                RemoteDownloadPlan.linkIn(
                    it.getCharSequenceExtra(Intent.EXTRA_TEXT)?.toString(),
                    it.getStringExtra(Intent.EXTRA_SUBJECT) ?: it.getStringExtra(Intent.EXTRA_TITLE),
                )
            }
            if (link == null) {
                Toast.makeText(applicationContext, getString(R.string.dl_share_no_link), Toast.LENGTH_SHORT).show()
            } else {
                runCatching {
                    startActivity(
                        Intent(this, MainActivity::class.java)
                            .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_SINGLE_TOP)
                            .putExtra(EXTRA_LINK, link),
                    )
                }
            }
        }
        finish()
        overridePendingTransition(0, 0)
    }

    companion object {
        /** On MainActivity's intent: the link to download on another device. */
        const val EXTRA_LINK = "com.mydomain.android.DOWNLOAD_LINK"
    }
}
