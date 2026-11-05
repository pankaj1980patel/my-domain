package com.mydomain.android

import android.app.PendingIntent
import android.appwidget.AppWidgetManager
import android.appwidget.AppWidgetProvider
import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.os.Bundle
import android.view.View
import android.widget.RemoteViews
import com.mydomain.android.core.Notes
import com.mydomain.android.core.NotesGlance
import com.mydomain.android.ffi.NoteItem
import org.json.JSONArray
import org.json.JSONObject

/**
 * The Notes home-screen widget: pinned notes first, then the most recent
 * ([NotesGlance] picks them), each its title and first line with a bell
 * when a reminder is still to come, as many as the widget's height has
 * room for. Tapping a note opens it in the app; "+" starts a new one; the
 * header opens the Notes tab.
 *
 * Whenever the notes change here (read at start, saved here, changed on
 * another device) [Notes] hands them to [sync], which keeps the rows in
 * preferences and redraws: the widget shows them while the app isn't
 * running too. Before the notes were ever read it asks to open the app.
 *
 * The colours are the home-screen widget's ([HomeWidget]), the Cosmic Blue
 * theme's, since a widget can't use the Compose theme.
 */
class NotesWidget : AppWidgetProvider() {
    override fun onUpdate(context: Context, manager: AppWidgetManager, ids: IntArray) {
        ids.forEach { id -> runCatching { manager.updateAppWidget(id, views(context, manager, id)) } }
    }

    /** Resized: as many rows as fit now. */
    override fun onAppWidgetOptionsChanged(context: Context, manager: AppWidgetManager, id: Int, options: Bundle) {
        runCatching { manager.updateAppWidget(id, views(context, manager, id)) }
    }

    companion object {
        /** On MainActivity's intent: open this note ([NEW_NOTE]: a new one). */
        const val ACTION_OPEN_NOTE = "com.mydomain.android.OPEN_NOTE"
        /** On MainActivity's intent: start a new note (the widget's "+", the
         *  Quick Settings tile, the launcher shortcut in res/xml/shortcuts.xml). */
        const val ACTION_NEW_NOTE = "com.mydomain.android.NEW_NOTE"
        /** On MainActivity's intent: show the Notes tab. */
        const val ACTION_SHOW_NOTES = "com.mydomain.android.SHOW_NOTES"

        private const val PREFS = "notes_widget"
        private const val KEY_ROWS = "rows"

        /** PendingIntent request codes: the header, "+", then one per row. */
        private const val CODE_HEAD = 7100
        private const val CODE_NEW = 7101
        private const val CODE_ROW = 7110

        private val ROW_IDS = intArrayOf(R.id.notes_row_0, R.id.notes_row_1, R.id.notes_row_2, R.id.notes_row_3, R.id.notes_row_4)
        private val TITLE_IDS = intArrayOf(
            R.id.notes_row_0_title, R.id.notes_row_1_title, R.id.notes_row_2_title, R.id.notes_row_3_title, R.id.notes_row_4_title,
        )
        private val LINE_IDS = intArrayOf(
            R.id.notes_row_0_line, R.id.notes_row_1_line, R.id.notes_row_2_line, R.id.notes_row_3_line, R.id.notes_row_4_line,
        )
        private val BELL_IDS = intArrayOf(
            R.id.notes_row_0_bell, R.id.notes_row_1_bell, R.id.notes_row_2_bell, R.id.notes_row_3_bell, R.id.notes_row_4_bell,
        )

        private val lock = Any()

        /** The notes as they are now (every note here): keep the rows and
         *  redraw if they changed. Any thread (call it off the main one). */
        fun sync(ctx: Context, notes: Collection<NoteItem>) {
            val app = ctx.applicationContext
            val rows = NotesGlance.pick(
                notes.map { NotesGlance.Note(it.id, it.title, it.body, it.pinned, it.updatedAt, it.remindAt, it.remindDone) },
            )
            val json = encode(rows)
            synchronized(lock) {
                val prefs = app.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
                if (prefs.getString(KEY_ROWS, null) == json) return
                prefs.edit().putString(KEY_ROWS, json).apply()
            }
            redraw(app)
        }

        /** Redraw every placed Notes widget. Any thread. */
        fun redraw(ctx: Context) {
            val app = ctx.applicationContext
            runCatching {
                val manager = AppWidgetManager.getInstance(app) ?: return
                val ids = manager.getAppWidgetIds(ComponentName(app, NotesWidget::class.java))
                ids.forEach { id -> manager.updateAppWidget(id, views(app, manager, id)) }
            }
        }

        private fun encode(rows: List<NotesGlance.Row>): String {
            val arr = JSONArray()
            rows.forEach { r ->
                arr.put(
                    JSONObject().put("id", r.id).put("line", r.line).put("bell", r.bell)
                        .apply { r.title?.let { put("title", it) } },
                )
            }
            return arr.toString()
        }

        /** The kept rows; null if the notes were never read here. */
        private fun rows(ctx: Context): List<NotesGlance.Row>? {
            val json = synchronized(lock) {
                ctx.getSharedPreferences(PREFS, Context.MODE_PRIVATE).getString(KEY_ROWS, null)
            } ?: return null
            val arr = runCatching { JSONArray(json) }.getOrNull() ?: return null
            return (0 until arr.length()).mapNotNull { i ->
                val o = arr.optJSONObject(i) ?: return@mapNotNull null
                val id = o.optString("id").ifEmpty { null } ?: return@mapNotNull null
                NotesGlance.Row(
                    id = id,
                    title = if (o.has("title")) o.optString("title") else null,
                    line = o.optString("line"),
                    bell = o.optBoolean("bell"),
                )
            }
        }

        /** How many rows this widget has room for. Phones report portrait
         *  as the min width and the max height. */
        private fun fits(manager: AppWidgetManager, id: Int): Int {
            val opts = runCatching { manager.getAppWidgetOptions(id) }.getOrNull()
            val h = opts?.getInt(AppWidgetManager.OPTION_APPWIDGET_MAX_HEIGHT, 0) ?: 0
            return NotesGlance.rowsFor(h)
        }

        /** The widget as it should look now. */
        private fun views(ctx: Context, manager: AppWidgetManager, widgetId: Int): RemoteViews {
            val v = RemoteViews(ctx.packageName, R.layout.notes_widget)
            v.setOnClickPendingIntent(R.id.notes_widget_head, activity(ctx, CODE_HEAD, showNotesIntent(ctx)))
            v.setOnClickPendingIntent(R.id.notes_widget_new, activity(ctx, CODE_NEW, newNoteIntent(ctx)))
            val rows = rows(ctx)
            val shown = rows.orEmpty().take(fits(manager, widgetId))
            for (i in ROW_IDS.indices) {
                val r = shown.getOrNull(i)
                if (r == null) {
                    v.setViewVisibility(ROW_IDS[i], View.GONE)
                    continue
                }
                v.setViewVisibility(ROW_IDS[i], View.VISIBLE)
                v.setTextViewText(TITLE_IDS[i], r.title ?: ctx.getString(R.string.notes_untitled))
                v.setTextViewText(LINE_IDS[i], r.line)
                v.setViewVisibility(LINE_IDS[i], if (r.line.isEmpty()) View.GONE else View.VISIBLE)
                v.setViewVisibility(BELL_IDS[i], if (r.bell) View.VISIBLE else View.GONE)
                v.setOnClickPendingIntent(ROW_IDS[i], activity(ctx, CODE_ROW + i, openNoteIntent(ctx, r.id)))
            }
            if (shown.isEmpty()) {
                v.setViewVisibility(R.id.notes_widget_empty, View.VISIBLE)
                v.setTextViewText(
                    R.id.notes_widget_empty,
                    ctx.getString(if (rows == null) R.string.notes_widget_open_app else R.string.notes_widget_empty),
                )
                v.setOnClickPendingIntent(
                    R.id.notes_widget_empty,
                    activity(ctx, CODE_HEAD, showNotesIntent(ctx)),
                )
            } else {
                v.setViewVisibility(R.id.notes_widget_empty, View.GONE)
            }
            return v
        }

        /** MainActivity, brought forward (or started) with this intent. */
        private fun activity(ctx: Context, code: Int, intent: Intent): PendingIntent =
            PendingIntent.getActivity(ctx, code, intent, PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE)

        /** Opens the app on the Notes tab (the list). */
        private fun showNotesIntent(ctx: Context): Intent =
            Intent(ctx, MainActivity::class.java).setAction(ACTION_SHOW_NOTES).addFlags(LAUNCH_FLAGS)

        /** Opens the app on a new note. */
        fun newNoteIntent(ctx: Context): Intent =
            Intent(ctx, MainActivity::class.java).setAction(ACTION_NEW_NOTE).addFlags(LAUNCH_FLAGS)

        /** Opens the app on note [id]. */
        fun openNoteIntent(ctx: Context, id: String): Intent =
            Intent(ctx, MainActivity::class.java).setAction(ACTION_OPEN_NOTE)
                .putExtra(NoteReminders.EXTRA_OPEN_NOTE, id).addFlags(LAUNCH_FLAGS)

        /** A new task if the app isn't open, else back to MainActivity (on
         *  top of its task) with the intent. */
        private const val LAUNCH_FLAGS =
            Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_CLEAR_TOP or Intent.FLAG_ACTIVITY_SINGLE_TOP

        /** What MainActivity was started with, for the UI: a note id,
         *  [Notes.NEW] or [Notes.LIST]; null if it isn't one of these. */
        fun noteRequest(intent: Intent?): String? = when (intent?.action) {
            ACTION_NEW_NOTE -> Notes.NEW
            ACTION_SHOW_NOTES -> Notes.LIST
            ACTION_OPEN_NOTE -> intent?.getStringExtra(NoteReminders.EXTRA_OPEN_NOTE)
            else -> null
        }
    }
}
