package com.mydomain.android

import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.os.Build
import android.os.Environment
import androidx.core.app.NotificationCompat
import com.mydomain.android.core.Core
import com.mydomain.android.ffi.SyncPairInfo
import java.io.File
import java.util.concurrent.ConcurrentHashMap

/**
 * The phone side of synced folders (the core's `features::folder_sync`).
 * The core does the syncing itself, on plain file paths, while NetService
 * runs; this picks where an accepted folder goes and says when a computer
 * asks to sync one.
 *
 * Where: the core needs a real path, so no Storage Access Framework tree
 * (writing through a tree Uri would mean a Kotlin file layer for every
 * read, write and scan). Instead:
 *  - Android 11 and later: the public Downloads/my-domain/Sync/<name>,
 *    which apps may write by path. Files this app writes there it can read
 *    back; files other apps put there may not be readable (they're then
 *    skipped, never deleted).
 *  - Android 10 and older: the app's own storage
 *    (Android/data/<app>/files/Sync/<name>).
 */
object SyncFolders {
    private const val SUBFOLDER = "my-domain"
    private const val SYNC = "Sync"

    /** Invitations already announced (once each per process). */
    private val announced: MutableSet<String> = ConcurrentHashMap.newKeySet()

    /** Synced folders can go in the public Downloads (Android 11+). */
    val publicPossible: Boolean get() = Build.VERSION.SDK_INT >= Build.VERSION_CODES.R

    /** The folder that holds the synced folders on this phone: the public
     *  Downloads/my-domain/Sync where possible, unless [appOnly]; else the
     *  app's own storage. */
    fun base(ctx: Context, appOnly: Boolean = false): File =
        if (publicPossible && !appOnly) {
            File(File(Environment.getExternalStoragePublicDirectory(Environment.DIRECTORY_DOWNLOADS), SUBFOLDER), SYNC)
        } else {
            ctx.getExternalFilesDir(SYNC) ?: File(ctx.filesDir, SYNC)
        }

    /** Where a synced folder called [name] goes: a folder of that name that
     *  doesn't exist yet (numbered if it does), so two never mix. */
    fun destination(ctx: Context, name: String, appOnly: Boolean = false): File {
        val clean = name.replace(Regex("[\\\\/:*?\"<>|\\p{Cntrl}]"), "").trim().trimStart('.').ifEmpty { "Synced folder" }
        val base = base(ctx, appOnly)
        var dir = File(base, clean)
        var n = 2
        while (dir.exists() && n < 1000) {
            dir = File(base, "$clean ($n)")
            n++
        }
        return dir
    }

    /** `sync-pairs`: a notice for each new invitation (from a computer). */
    fun onPairs(ctx: Context, pairs: List<SyncPairInfo>) {
        for (p in pairs) {
            if (p.state != "invited" || !announced.add(p.id)) continue
            val device = Core.peerName(p.peer)
            val open = PendingIntent.getActivity(
                ctx, p.id.hashCode(),
                Intent(ctx, MainActivity::class.java)
                    .putExtra(PhoneLocation.EXTRA_OPEN_SETTINGS, true)
                    .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_SINGLE_TOP),
                PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
            )
            val n = NotificationCompat.Builder(ctx, FileActions.CH_FILES)
                .setSmallIcon(android.R.drawable.stat_notify_sync)
                .setContentTitle(ctx.getString(R.string.sync_invite_title, device))
                .setContentText(ctx.getString(R.string.sync_invite_body, p.name))
                .setAutoCancel(true)
                .setContentIntent(open)
                .build()
            runCatching {
                ctx.getSystemService(android.app.NotificationManager::class.java)
                    ?.notify(0x5000_0000 or (p.id.hashCode() and 0x0FFF_FFFF), n)
            }
        }
    }
}
