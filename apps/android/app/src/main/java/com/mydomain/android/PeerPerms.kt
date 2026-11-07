package com.mydomain.android

import android.app.Activity
import android.app.AlertDialog
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.util.Log
import android.widget.Toast
import androidx.core.app.NotificationCompat
import com.mydomain.android.core.Core
import com.mydomain.android.core.PermsPlan

/**
 * Per-device permissions on the phone (mdcore `features::perms`).
 *
 *  - A computer asks to see this screen and isn't allowed: the core
 *    refuses it and asks here (`permission_ask`): "<Mac> wants to see this
 *    screen — Allow once / Always / Don't allow" (a dialog while the app is
 *    on screen, else a notification that opens it). Allowing goes on to
 *    Android's own prompt, as always.
 *  - A computer refused what this phone asked (`permission_refused`): its
 *    touchpad or its screen. Say why and where to allow it.
 */
object PeerPerms {
    private const val TAG = "mydomain_perms"
    const val NOTE_ASK = 7311
    private val said = PermsPlan.Throttle()
    private val main = Handler(Looper.getMainLooper())

    /** Whether computer [node] may see this screen always. */
    fun screenAllowed(node: String): Boolean =
        runCatching { Core.engine()?.peerPermission(node, PermsPlan.SCREEN) == true }.getOrDefault(false)

    /** The switch on the computer's page. */
    fun setScreenAllowed(node: String, on: Boolean): Boolean =
        runCatching { Core.engine()?.setPeerPermission(node, PermsPlan.SCREEN, on) }
            .onFailure { Log.w(TAG, "set: $it") }.isSuccess

    /** You chose to share this screen with [node]: allowed for this time
     *  (unless it's allowed always). */
    fun allowForSharing(node: String) {
        val answer = PermsPlan.shareAnswer(screenAllowed(node)) ?: return
        runCatching { Core.engine()?.permissionAnswer(node, PermsPlan.SCREEN, answer) }
            .onFailure { Log.w(TAG, "allow once: $it") }
    }

    /** The core refused computer [from] something this phone decides
     *  ([perm]): ask the owner. */
    fun onAsk(ctx: Context, from: String, perm: String) {
        if (!PermsPlan.asksHere(perm) || from.isEmpty()) return
        val app = ctx.applicationContext
        ScreenShare.channel(app)
        val ask = Intent(app, PermissionAskActivity::class.java)
            .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
            .putExtra(PermissionAskActivity.EXTRA_NODE, from)
            .putExtra(PermissionAskActivity.EXTRA_PERM, perm)
        if (Core.visible && runCatching { app.startActivity(ask) }.isSuccess) return
        val pi = PendingIntent.getActivity(app, NOTE_ASK, ask, PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
        val n = NotificationCompat.Builder(app, "screen_share")
            .setSmallIcon(android.R.drawable.ic_menu_view)
            .setContentTitle(app.getString(R.string.perm_ask_screen, Core.peerName(from)))
            .setContentText(app.getString(R.string.perm_ask_body))
            .setPriority(NotificationCompat.PRIORITY_HIGH)
            .setAutoCancel(true)
            .setContentIntent(pi)
            .build()
        runCatching { app.getSystemService(NotificationManager::class.java)?.notify(NOTE_ASK, n) }
            .onFailure { Log.w(TAG, "can't ask: $it") }
    }

    /** The owner answered [answer] for [node] and [perm]. */
    fun answer(act: Activity, node: String, perm: String, answer: String) {
        runCatching { Core.engine()?.permissionAnswer(node, perm, answer) }
            .onFailure { Log.w(TAG, "answer: $it") }
        if (PermsPlan.startsCapture(perm, answer)) {
            // Android's own prompt, then the frames go to it.
            act.startActivity(
                Intent(act, ScreenConsentActivity::class.java)
                    .putExtra(ScreenConsentActivity.EXTRA_NODE, node),
            )
        }
    }

    /** Computer [from] refused [perm] to this phone: say why, once in a
     *  while. */
    fun onRefused(ctx: Context, from: String, perm: String) {
        if (!PermsPlan.known(perm) || !said.due(from, perm, System.currentTimeMillis())) return
        val name = Core.peerName(from)
        val res = when (perm) {
            PermsPlan.CONTROL -> R.string.perm_refused_control
            PermsPlan.SCREEN -> R.string.perm_refused_screen
            else -> R.string.perm_refused_kvm
        }
        val text = ctx.getString(res, name)
        main.post { Toast.makeText(ctx.applicationContext, text, Toast.LENGTH_LONG).show() }
    }
}

/** "<Mac> wants to see this screen": Allow once / Always / Don't allow. */
class PermissionAskActivity : Activity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        getSystemService(NotificationManager::class.java)?.cancel(PeerPerms.NOTE_ASK)
        val node = intent?.getStringExtra(EXTRA_NODE).orEmpty()
        val perm = intent?.getStringExtra(EXTRA_PERM).orEmpty()
        if (node.isEmpty() || !PermsPlan.asksHere(perm)) return finish()
        AlertDialog.Builder(this, android.R.style.Theme_DeviceDefault_Dialog_Alert)
            .setTitle(getString(R.string.perm_ask_screen, Core.peerName(node)))
            .setMessage(getString(R.string.perm_ask_body))
            .setPositiveButton(R.string.perm_always) { _, _ ->
                PeerPerms.answer(this, node, perm, PermsPlan.ALWAYS)
                finish()
            }
            .setNeutralButton(R.string.perm_once) { _, _ ->
                PeerPerms.answer(this, node, perm, PermsPlan.ONCE)
                finish()
            }
            .setNegativeButton(R.string.perm_deny) { _, _ ->
                PeerPerms.answer(this, node, perm, PermsPlan.DENY)
                finish()
            }
            // Dismissed: no answer; it asks again next time.
            .setOnCancelListener { finish() }
            .show()
    }

    companion object {
        const val EXTRA_NODE = "node"
        const val EXTRA_PERM = "perm"
    }
}
