package com.mydomain.android

import android.app.NotificationManager
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.util.Log
import com.mydomain.android.core.Core
import org.json.JSONObject

/**
 * Do Not Disturb follows the phone (mdcore `features::dnd`): tell the core
 * whenever this phone goes into or out of Do Not Disturb, and it tells your
 * computers. Read-only: the interruption filter can be read, and its change
 * broadcast received, without any permission. Changing it would need
 * notification-policy access, which this app never asks for.
 *
 * It's in the status too (`dnd: true|false`, see [state]), and a change
 * pushes a fresh status, so a computer can show it and stay quiet meanwhile.
 */
object DndWatch {
    private const val TAG = "mydomain_dnd"
    private var receiver: BroadcastReceiver? = null
    /** The last state read (null until read, or while the filter is unknown). */
    @Volatile private var last: Boolean? = null

    fun start(ctx: Context) {
        val c = ctx.applicationContext
        val r = object : BroadcastReceiver() {
            override fun onReceive(context: Context, intent: Intent) = push(context)
        }
        receiver = r
        // Sent only to receivers registered at run time (not in the manifest).
        runCatching { c.registerReceiver(r, IntentFilter(NotificationManager.ACTION_INTERRUPTION_FILTER_CHANGED)) }
            .onFailure { Log.w(TAG, "can't watch Do Not Disturb", it) }
        push(c)
    }

    fun stop(ctx: Context) {
        receiver?.let { runCatching { ctx.applicationContext.unregisterReceiver(it) } }
        receiver = null
    }

    /** Read the filter now and hand it to the core (it sends only changes). */
    fun push(ctx: Context) {
        val nm = ctx.getSystemService(NotificationManager::class.java) ?: return
        onFilter(nm.currentInterruptionFilter)
    }

    /** The interruption filter is [filter] (also from the notification listener). */
    fun onFilter(filter: Int) {
        val on = isOn(filter) ?: return // unknown: say nothing
        Core.engine()?.let { e -> runCatching { e.setDnd(on) } }
        if (last != on) {
            last = on
            // A fresh status: it's pushed only when something in it changed.
            Controls.onChange()
        }
    }

    /** Put `dnd` in the status (left out while the filter is unknown). Read
     *  fresh, so it's right even before the first broadcast. */
    fun state(ctx: Context, o: JSONObject) {
        val nm = ctx.getSystemService(NotificationManager::class.java) ?: return
        val filter = runCatching { nm.currentInterruptionFilter }.getOrNull() ?: return
        isOn(filter)?.let { o.put("dnd", it) }
    }

    /** Anything but "all notifications" is Do Not Disturb; null: unknown. */
    private fun isOn(filter: Int): Boolean? = when (filter) {
        NotificationManager.INTERRUPTION_FILTER_PRIORITY,
        NotificationManager.INTERRUPTION_FILTER_ALARMS,
        NotificationManager.INTERRUPTION_FILTER_NONE -> true
        NotificationManager.INTERRUPTION_FILTER_ALL -> false
        else -> null
    }
}
