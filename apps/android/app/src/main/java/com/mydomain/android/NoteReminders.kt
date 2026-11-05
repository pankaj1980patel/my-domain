package com.mydomain.android

import android.app.AlarmManager
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.net.Uri
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.provider.Settings
import androidx.core.app.NotificationCompat
import com.mydomain.android.core.Core
import com.mydomain.android.core.Notes
import com.mydomain.android.core.ReminderPlan
import com.mydomain.android.ffi.NoteItem
import org.json.JSONArray
import org.json.JSONObject

/**
 * Reminders on notes, on this phone (the core's `features::notes` syncs
 * them; [ReminderPlan] has the rules). A note's reminder shows on every
 * device that has it, whichever is on: here as a notification, "<title>"
 * with the first line of the note, with Done, 10 min and 1 hour (Android
 * shows three buttons; the note itself, a tap away, has "Tomorrow 9:00").
 *
 *  - The notes' reminders are kept here ([sync], whenever the notes change:
 *    read at start, saved here, or changed on another device), so the
 *    alarm can show one while the app isn't running.
 *  - One alarm, for the soonest reminder: exact
 *    (`setExactAndAllowWhileIdle`) when Android lets this app, else
 *    inexact (`setAndAllowWhileIdle`, may be some minutes late), which the
 *    note's reminder says, with a button to allow exact alarms (Android
 *    12+ asks you; this app never asks behind your back).
 *  - Set again after a restart (the boot receiver) and whenever the notes
 *    change.
 *  - Done or a new time (here or on another device, once it syncs) takes
 *    the notification down. Done and Snooze from the notification save the
 *    note through the core; when it isn't running yet they wait here and
 *    go once it is ([flushPending]).
 */
object NoteReminders {
    const val CHANNEL = "note_reminders"
    const val ACTION_ALARM = "com.mydomain.android.NOTE_REMINDER_ALARM"
    const val ACTION_DONE = "com.mydomain.android.NOTE_REMINDER_DONE"
    const val ACTION_SNOOZE = "com.mydomain.android.NOTE_REMINDER_SNOOZE"
    const val EXTRA_ID = "id"
    const val EXTRA_SNOOZE = "snooze"
    /** MainActivity: open this note. */
    const val EXTRA_OPEN_NOTE = "open_note"

    private const val PREFS = "note_reminders"
    private const val KEY_NOTES = "notes"
    private const val KEY_SHOWN = "shown"
    private const val KEY_PENDING = "pending"

    private val main = Handler(Looper.getMainLooper())

    private fun prefs(ctx: Context) = ctx.getSharedPreferences(PREFS, Context.MODE_PRIVATE)

    fun createChannel(ctx: Context) {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.O) return
        val ch = NotificationChannel(CHANNEL, ctx.getString(R.string.channel_note_reminders), NotificationManager.IMPORTANCE_HIGH)
        runCatching { ctx.getSystemService(NotificationManager::class.java)?.createNotificationChannel(ch) }
    }

    /** Android lets this app set exact alarms (always before Android 12,
     *  and for an app targeting older Androids). */
    fun exactAllowed(ctx: Context): Boolean {
        if (!Sdk.rules(Build.VERSION_CODES.S)) return true
        val am = ctx.getSystemService(AlarmManager::class.java) ?: return false
        return am.canScheduleExactAlarms()
    }

    /** Open Android's "Alarms & reminders" page for this app (you choose). */
    fun askExact(ctx: Context) {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.S) return
        runCatching {
            ctx.startActivity(
                Intent(Settings.ACTION_REQUEST_SCHEDULE_EXACT_ALARM, Uri.parse("package:${ctx.packageName}"))
                    .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK),
            )
        }
    }

    // --- what's kept -----------------------------------------------------------

    private fun entries(ctx: Context): List<ReminderPlan.Entry> {
        val arr = runCatching { JSONArray(prefs(ctx).getString(KEY_NOTES, "[]")) }.getOrNull() ?: return emptyList()
        return (0 until arr.length()).mapNotNull { i ->
            val o = arr.optJSONObject(i) ?: return@mapNotNull null
            ReminderPlan.Entry(
                id = o.optString("id").ifEmpty { return@mapNotNull null },
                at = if (o.has("at")) o.optLong("at") else null,
                done = o.optBoolean("done"),
                title = o.optString("title"),
                line = o.optString("line"),
            )
        }
    }

    private fun saveEntries(ctx: Context, list: List<ReminderPlan.Entry>) {
        val arr = JSONArray()
        list.forEach { e ->
            arr.put(
                JSONObject().put("id", e.id).put("done", e.done).put("title", e.title).put("line", e.line)
                    .apply { e.at?.let { put("at", it) } },
            )
        }
        prefs(ctx).edit().putString(KEY_NOTES, arr.toString()).apply()
    }

    private fun readMap(ctx: Context, key: String): MutableMap<String, String> {
        val o = runCatching { JSONObject(prefs(ctx).getString(key, "{}")!!) }.getOrNull() ?: return HashMap()
        return o.keys().asSequence().associateWith { o.optString(it) }.toMutableMap()
    }

    private fun writeMap(ctx: Context, key: String, m: Map<String, String>) {
        prefs(ctx).edit().putString(key, JSONObject(m).toString()).apply()
    }

    private fun entryOf(n: NoteItem) =
        ReminderPlan.Entry(n.id, n.remindAt, n.remindDone, n.title, ReminderPlan.firstLine(n.body))

    // --- the plan ----------------------------------------------------------------

    /** The notes as they are now (every note here): kept for the alarm, and
     *  looked at. Any thread. */
    fun sync(ctx: Context, notes: Collection<NoteItem>) {
        saveEntries(ctx, notes.filter { it.remindAt != null }.map(::entryOf))
        check(ctx)
    }

    /** Show what's due, take down what's no longer, and set the alarm for
     *  the next one. */
    @Synchronized
    fun check(ctx: Context) {
        val list = entries(ctx)
        val shown = readMap(ctx, KEY_SHOWN).mapNotNull { (k, v) -> v.toLongOrNull()?.let { k to it } }.toMap()
        val plan = ReminderPlan.plan(list, shown, System.currentTimeMillis())
        val nm = ctx.getSystemService(NotificationManager::class.java)
        plan.dismiss.forEach { runCatching { nm?.cancel(notifId(it)) } }
        plan.show.forEach { id -> list.firstOrNull { it.id == id }?.let { post(ctx, it) } }
        if (plan.shown != shown) writeMap(ctx, KEY_SHOWN, plan.shown.mapValues { it.value.toString() })
        schedule(ctx, plan.next)
    }

    private fun notifId(id: String) = "note|$id".hashCode()

    private fun schedule(ctx: Context, at: Long?) {
        val am = ctx.getSystemService(AlarmManager::class.java) ?: return
        val pi = PendingIntent.getBroadcast(
            ctx, 0,
            Intent(ctx, NoteReminderReceiver::class.java).setAction(ACTION_ALARM),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
        if (at == null) {
            am.cancel(pi)
            return
        }
        runCatching {
            if (exactAllowed(ctx)) am.setExactAndAllowWhileIdle(AlarmManager.RTC_WAKEUP, at, pi)
            else am.setAndAllowWhileIdle(AlarmManager.RTC_WAKEUP, at, pi)
        }.onFailure {
            // Exact taken away meanwhile: inexact still comes.
            runCatching { am.setAndAllowWhileIdle(AlarmManager.RTC_WAKEUP, at, pi) }
        }
    }

    private fun action(ctx: Context, id: String, action: String, snooze: ReminderPlan.Snooze? = null): PendingIntent {
        val i = Intent(ctx, NoteReminderReceiver::class.java).setAction(action).putExtra(EXTRA_ID, id)
        snooze?.let { i.putExtra(EXTRA_SNOOZE, it.name) }
        return PendingIntent.getBroadcast(
            ctx, "$action|$id|${snooze?.name}".hashCode(), i,
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
    }

    /** Show one (requires POST_NOTIFICATIONS on Android 13+; a no-op
     *  without). */
    private fun post(ctx: Context, e: ReminderPlan.Entry) {
        createChannel(ctx)
        val open = PendingIntent.getActivity(
            ctx, notifId(e.id),
            Intent(ctx, MainActivity::class.java).putExtra(EXTRA_OPEN_NOTE, e.id)
                .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_SINGLE_TOP),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
        val title = e.title.ifBlank { ctx.getString(R.string.note_reminder_untitled) }
        val n = NotificationCompat.Builder(ctx, CHANNEL)
            .setSmallIcon(android.R.drawable.ic_popup_reminder)
            .setContentTitle(title)
            .setContentText(e.line)
            .setStyle(NotificationCompat.BigTextStyle().bigText(e.line))
            .setCategory(NotificationCompat.CATEGORY_REMINDER)
            .setPriority(NotificationCompat.PRIORITY_HIGH)
            .setWhen(e.at ?: System.currentTimeMillis())
            .setShowWhen(true)
            .setAutoCancel(true)
            .setContentIntent(open)
            .addAction(0, ctx.getString(R.string.note_reminder_done), action(ctx, e.id, ACTION_DONE))
            .addAction(0, ctx.getString(R.string.note_reminder_snooze_10m), action(ctx, e.id, ACTION_SNOOZE, ReminderPlan.Snooze.TEN_MIN))
            .addAction(0, ctx.getString(R.string.note_reminder_snooze_1h), action(ctx, e.id, ACTION_SNOOZE, ReminderPlan.Snooze.HOUR))
            .build()
        runCatching { ctx.getSystemService(NotificationManager::class.java)?.notify(notifId(e.id), n) }
    }

    // --- Done and Snooze -----------------------------------------------------------

    /** Mark it Done: here at once, on your other devices as they sync. */
    fun done(ctx: Context, id: String) {
        val e = entries(ctx).firstOrNull { it.id == id }
        change(ctx, id, e?.at, true)
    }

    /** Remind again later. */
    fun snooze(ctx: Context, id: String, s: ReminderPlan.Snooze) {
        change(ctx, id, ReminderPlan.snoozeUntil(s, System.currentTimeMillis()), false)
    }

    /** The reminder kept here changes at once (the notification goes, the
     *  alarm moves); the note is saved through the core, now or once it
     *  runs. */
    private fun change(ctx: Context, id: String, at: Long?, done: Boolean) {
        saveEntries(ctx, entries(ctx).map { if (it.id == id) it.copy(at = at, done = done) else it })
        check(ctx)
        synchronized(this) {
            val pending = readMap(ctx, KEY_PENDING)
            pending[id] = JSONObject().put("done", done).apply { at?.let { put("at", it) } }.toString()
            writeMap(ctx, KEY_PENDING, pending)
        }
        Thread { if (!flushPending(ctx)) startService(ctx) }.start()
    }

    private fun startService(ctx: Context) {
        val svc = Intent(ctx, NetService::class.java)
        runCatching {
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) ctx.startForegroundService(svc) else ctx.startService(svc)
        }
    }

    /** Save the Done and Snooze made from notifications while the core
     *  wasn't running. True if none is left. Off the main thread. */
    fun flushPending(ctx: Context): Boolean {
        val engine = Core.engine() ?: return false
        val pending = synchronized(this) { readMap(ctx, KEY_PENDING) }
        if (pending.isEmpty()) return true
        for ((id, json) in pending) {
            val o = runCatching { JSONObject(json) }.getOrNull()
            val n = runCatching { engine.noteGet(id) }.getOrNull()
            if (o != null && n != null) {
                val at = if (o.has("at")) o.optLong("at") else null
                val saved = runCatching {
                    engine.noteSave(id, n.title, n.body, n.pinned, n.checklist, n.color, at, o.optBoolean("done"))
                }.getOrNull()
                if (saved != null) main.post { Notes.applySaved(saved) }
                else continue
            }
            synchronized(this) {
                val now = readMap(ctx, KEY_PENDING)
                if (now[id] == json) {
                    now.remove(id)
                    writeMap(ctx, KEY_PENDING, now)
                }
            }
        }
        return synchronized(this) { readMap(ctx, KEY_PENDING).isEmpty() }
    }
}

/** The reminder alarm, and a reminder notification's Done and Snooze. */
class NoteReminderReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        val ctx = context.applicationContext
        val id = intent.getStringExtra(NoteReminders.EXTRA_ID)
        val pending = goAsync()
        Thread {
            try {
                when (intent.action) {
                    NoteReminders.ACTION_ALARM -> NoteReminders.check(ctx)
                    NoteReminders.ACTION_DONE -> if (id != null) NoteReminders.done(ctx, id)
                    NoteReminders.ACTION_SNOOZE -> {
                        val s = runCatching { ReminderPlan.Snooze.valueOf(intent.getStringExtra(NoteReminders.EXTRA_SNOOZE) ?: "") }.getOrNull()
                        if (id != null && s != null) NoteReminders.snooze(ctx, id, s)
                    }
                }
            } finally {
                pending.finish()
            }
        }.start()
    }
}
