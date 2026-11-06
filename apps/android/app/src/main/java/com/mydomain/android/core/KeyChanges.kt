package com.mydomain.android.core

import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.core.app.NotificationCompat
import com.mydomain.android.DeviceActions
import com.mydomain.android.MainActivity
import com.mydomain.android.R
import com.mydomain.android.ShareTargets
import org.json.JSONArray

/**
 * Devices whose security key changed (reinstalled, data cleared, or someone
 * else using their id): the core refuses their direct connections until
 * the user trusts the new key. The list ([list]) is read at start and again
 * when the core says a key changed (`key_changed`, once per new key: a
 * notification goes then too) and after the user decides. Never trusted by
 * itself: only "Trust the new key", confirmed, trusts the key shown.
 */
object KeyChanges {
    var list by mutableStateOf<List<KeyChangeInfo>>(emptyList())
        private set

    /** Read the list again (off the main thread). */
    suspend fun load() {
        val json = Core.call { e -> e.keyChanges() }
        list = parse(JSONArray(json))
    }

    /** Trust [c]'s new key: the one whose fingerprint was shown. Throws the core's error. */
    suspend fun trust(c: KeyChangeInfo) {
        try {
            Core.call { e -> e.trustNewKey(c.nodeId, c.newFingerprint) }
        } finally {
            runCatching { load() }
        }
        runCatching { Core.app.getSystemService(NotificationManager::class.java)?.cancel(KeyChangePlan.notificationId(c.nodeId)) }
    }

    /** "Not now": keep refusing it, stop asking. */
    suspend fun dismiss(node: String) {
        Core.call { e -> e.dismissKeyChange(node) }
        runCatching { load() }
        runCatching { Core.app.getSystemService(NotificationManager::class.java)?.cancel(KeyChangePlan.notificationId(node)) }
    }

    /** The name to show for [c]. */
    fun nameOf(c: KeyChangeInfo): String =
        KeyChangePlan.nameOf(c) { n -> if (Core.peers.any { it.nodeId == n }) Core.peerName(n) else null }

    /** A `key_changed` event: tell the user, once (the core sends it once per key). */
    fun onChanged(ctx: Context, node: String, coreName: String) {
        val app = ctx.applicationContext
        val name = KeyChangePlan.nameOf(KeyChangeInfo(node, coreName, "", "")) { n ->
            if (Core.peers.any { it.nodeId == n }) Core.peerName(n) else null
        }
        val open = PendingIntent.getActivity(
            app, KeyChangePlan.notificationId(node),
            Intent(app, MainActivity::class.java).setAction(Intent.ACTION_VIEW)
                .putExtra(ShareTargets.EXTRA_OPEN_DEVICE, node)
                .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_SINGLE_TOP),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
        val body = app.getString(R.string.key_change_body)
        val n = NotificationCompat.Builder(app, DeviceActions.CH_LINKS)
            .setSmallIcon(android.R.drawable.ic_dialog_alert)
            .setContentTitle(app.getString(R.string.key_change_title, name))
            .setContentText(body)
            .setStyle(NotificationCompat.BigTextStyle().bigText(body))
            .setPriority(NotificationCompat.PRIORITY_HIGH)
            .setContentIntent(open)
            .setAutoCancel(true)
            .build()
        runCatching { app.getSystemService(NotificationManager::class.java)?.notify(KeyChangePlan.notificationId(node), n) }
    }

    fun parse(a: JSONArray): List<KeyChangeInfo> = (0 until a.length()).mapNotNull { i ->
        val o = a.optJSONObject(i) ?: return@mapNotNull null
        KeyChangeInfo(
            nodeId = o.optString("node_id"),
            name = o.optString("name"),
            oldFingerprint = o.optString("old_fingerprint"),
            newFingerprint = o.optString("new_fingerprint"),
            firstAgoS = o.optLong("first_ago_s"),
            lastAgoS = o.optLong("last_ago_s"),
            attempts = o.optInt("attempts"),
            dismissed = o.optBoolean("dismissed"),
        )
    }
}
