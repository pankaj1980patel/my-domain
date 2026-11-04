package com.mydomain.android

import android.app.PendingIntent
import android.appwidget.AppWidgetManager
import android.appwidget.AppWidgetProvider
import android.content.BroadcastReceiver
import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.os.Handler
import android.os.Looper
import android.os.SystemClock
import android.view.View
import android.widget.RemoteViews
import com.mydomain.android.core.Core

/**
 * Home-screen widget: your computer's name, whether it's connected and its
 * battery, with Send clipboard, Lock and Ring. The computer is the Quick
 * Settings tiles' one ([QuickTarget]) and the buttons do what the tiles do
 * ([QuickActions]).
 *
 * Core calls [refresh] when links or a device's status change; the widget
 * is redrawn at most every few seconds. Without a running engine (the app
 * was never opened, or Android stopped it) it asks to open the app.
 */
class HomeWidget : AppWidgetProvider() {
    override fun onUpdate(context: Context, manager: AppWidgetManager, ids: IntArray) {
        Core.init(context)
        manager.updateAppWidget(ids, views(context))
    }

    companion object {
        private const val GAP_MS = 3_000L
        private val main = Handler(Looper.getMainLooper())
        private var last = 0L
        private var pending = false

        /** Redraw every placed widget soon (at most once per [GAP_MS]; the
         *  last call always lands). Safe from any thread. */
        fun refresh(context: Context) {
            val app = context.applicationContext
            main.post {
                if (pending) return@post
                val wait = last + GAP_MS - SystemClock.elapsedRealtime()
                if (wait <= 0) {
                    push(app)
                } else {
                    pending = true
                    main.postDelayed({ pending = false; push(app) }, wait)
                }
            }
        }

        private fun push(app: Context) {
            last = SystemClock.elapsedRealtime()
            val manager = AppWidgetManager.getInstance(app)
            val ids = manager.getAppWidgetIds(ComponentName(app, HomeWidget::class.java))
            if (ids.isEmpty()) return
            runCatching { manager.updateAppWidget(ids, views(app)) }
        }

        /** The widget as it should look now. */
        private fun views(ctx: Context): RemoteViews {
            val v = RemoteViews(ctx.packageName, R.layout.home_widget)
            v.setOnClickPendingIntent(R.id.widget_root, openApp(ctx))
            val node = if (Core.engine() != null) QuickTarget.computer(ctx) else null
            if (node == null) {
                v.setTextViewText(R.id.widget_name, ctx.getString(R.string.app_name))
                v.setTextViewText(
                    R.id.widget_status,
                    ctx.getString(if (Core.engine() == null) R.string.widget_open_app else R.string.widget_no_computer),
                )
                v.setTextColor(R.id.widget_status, ctx.getColor(R.color.widget_text_muted))
                v.setViewVisibility(R.id.widget_buttons, View.GONE)
                return v
            }
            val status = Core.deviceStatus[node]
            val linked = Core.links.containsKey(node)
            val state = ctx.getString(if (linked) R.string.widget_connected else R.string.widget_not_connected)
            v.setTextViewText(R.id.widget_name, QuickTarget.name(node))
            v.setTextViewText(
                R.id.widget_status,
                status?.battery?.let { ctx.getString(R.string.widget_battery, state, it) } ?: state,
            )
            v.setTextColor(R.id.widget_status, ctx.getColor(if (linked) R.color.widget_ok else R.color.widget_text_muted))
            v.setViewVisibility(R.id.widget_buttons, View.VISIBLE)
            v.setOnClickPendingIntent(
                R.id.widget_clip,
                PendingIntent.getActivity(ctx, 1, QuickActions.clipIntent(ctx), PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE),
            )
            // Before any status came in this run, Lock is offered: the
            // computer was remembered for taking it.
            val canLock = status == null || "lock" in status.actions
            v.setViewVisibility(R.id.widget_lock, if (canLock) View.VISIBLE else View.GONE)
            v.setOnClickPendingIntent(R.id.widget_lock, action(ctx, HomeWidgetActionReceiver.LOCK, 2))
            // Every device can be rung.
            v.setOnClickPendingIntent(R.id.widget_ring, action(ctx, HomeWidgetActionReceiver.RING, 3))
            return v
        }

        private fun openApp(ctx: Context): PendingIntent {
            val open = Intent(ctx, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
            return PendingIntent.getActivity(ctx, 0, open, PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE)
        }

        private fun action(ctx: Context, name: String, code: Int): PendingIntent {
            val i = Intent(ctx, HomeWidgetActionReceiver::class.java).setAction(name)
            return PendingIntent.getBroadcast(ctx, code, i, PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE)
        }
    }
}

/** The widget's Lock and Ring buttons (not exported: only the widget's
 *  own PendingIntents reach it). */
class HomeWidgetActionReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        Core.init(context)
        when (intent.action) {
            LOCK -> QuickActions.lock(context)
            RING -> QuickActions.ring(context)
        }
    }

    companion object {
        const val LOCK = "com.mydomain.android.widget.LOCK"
        const val RING = "com.mydomain.android.widget.RING"
    }
}
