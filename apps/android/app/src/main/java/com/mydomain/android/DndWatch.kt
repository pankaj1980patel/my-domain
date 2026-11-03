package com.mydomain.android

import android.app.NotificationManager
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.util.Log
import com.mydomain.android.core.Core

/**
 * Do Not Disturb follows the phone (mdcore `features::dnd`): tell the core
 * whenever this phone goes into or out of Do Not Disturb, and it tells your
 * computers. Read-only: the interruption filter can be read, and its change
 * broadcast received, without any permission. Changing it would need
 * notification-policy access, which this app never asks for.
 */
object DndWatch {
    private const val TAG = "mydomain_dnd"
    private var receiver: BroadcastReceiver? = null

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
        val on = when (filter) {
            NotificationManager.INTERRUPTION_FILTER_PRIORITY,
            NotificationManager.INTERRUPTION_FILTER_ALARMS,
            NotificationManager.INTERRUPTION_FILTER_NONE -> true
            NotificationManager.INTERRUPTION_FILTER_ALL -> false
            else -> return // unknown: say nothing
        }
        Core.engine()?.let { e -> runCatching { e.setDnd(on) } }
    }
}
