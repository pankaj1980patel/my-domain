package com.mydomain.android

import android.app.AlarmManager
import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.media.AudioAttributes
import android.media.RingtoneManager
import android.os.Build
import android.os.Handler
import android.os.Looper
import androidx.core.app.NotificationCompat
import com.mydomain.android.core.Core
import com.mydomain.android.core.TimerPlan
import com.mydomain.android.core.Timers
import com.mydomain.android.ffi.TimerItem
import org.json.JSONArray
import org.json.JSONObject

/**
 * Timers on this phone (the core's `features::timers` syncs them;
 * [TimerPlan] has the rules).
 *
 *  - Every running or paused timer shows in a quiet notification with the
 *    time left (a countdown Android keeps going by itself), Pause or
 *    Resume, and +1 min.
 *  - A timer that rings on this phone rings when it ends: a notification on
 *    the alarm channel (your alarm sound, as an alarm, so it rings even
 *    while notifications are silenced), repeating until Stop, +1 min, or
 *    the timer is stopped on another device (it syncs here), at most
 *    [RING_FOR_MS].
 *  - The timers are kept here ([sync], whenever they change: read at
 *    start, changed here, or on another device), so the alarm can ring
 *    while the app isn't running. One alarm, for the soonest end: exact
 *    when Android lets this app (the setting note reminders use too), else
 *    inexact. Set again after a restart (the boot receiver).
 *  - Stop, +1 min, Pause and Resume from a notification change the timer
 *    kept here at once, and through the core now or once it runs
 *    ([flushPending]).
 */
object TimerAlarms {
    const val CHANNEL_RUNNING = "timers"
    const val CHANNEL_ALARM = "timer_alarm"
    const val ACTION_ALARM = "com.mydomain.android.TIMER_ALARM"
    const val ACTION_DO = "com.mydomain.android.TIMER_DO"
    const val EXTRA_ID = "id"
    const val EXTRA_OP = "op"
    /** MainActivity: open the Timers screen. */
    const val EXTRA_OPEN_TIMERS = "open_timers"
    /** A ringing timer gives up by itself after this long. */
    const val RING_FOR_MS = 5L * 60 * 1000
    private const val MINUTE = 60_000L

    private const val PREFS = "timer_alarms"
    private const val KEY_TIMERS = "timers"
    private const val KEY_ME = "me"
    private const val KEY_RUNG = "rung"
    private const val KEY_SHOWN = "shown"
    private const val KEY_PENDING = "pending"

    private val main = Handler(Looper.getMainLooper())

    private fun prefs(ctx: Context) = ctx.getSharedPreferences(PREFS, Context.MODE_PRIVATE)

    fun createChannels(ctx: Context) {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.O) return
        val nm = ctx.getSystemService(NotificationManager::class.java) ?: return
        val running = NotificationChannel(CHANNEL_RUNNING, ctx.getString(R.string.channel_timers), NotificationManager.IMPORTANCE_LOW)
        val alarm = NotificationChannel(CHANNEL_ALARM, ctx.getString(R.string.channel_timer_alarm), NotificationManager.IMPORTANCE_HIGH).apply {
            val sound = RingtoneManager.getDefaultUri(RingtoneManager.TYPE_ALARM)
                ?: RingtoneManager.getDefaultUri(RingtoneManager.TYPE_NOTIFICATION)
            setSound(
                sound,
                AudioAttributes.Builder()
                    .setUsage(AudioAttributes.USAGE_ALARM)
                    .setContentType(AudioAttributes.CONTENT_TYPE_SONIFICATION)
                    .build(),
            )
            enableVibration(true)
            setBypassDnd(true)
        }
        runCatching { nm.createNotificationChannel(running) }
        runCatching { nm.createNotificationChannel(alarm) }
    }

    // --- what's kept -----------------------------------------------------------

    private fun entries(ctx: Context): List<TimerPlan.Entry> {
        val arr = runCatching { JSONArray(prefs(ctx).getString(KEY_TIMERS, "[]")) }.getOrNull() ?: return emptyList()
        return (0 until arr.length()).mapNotNull { i ->
            val o = arr.optJSONObject(i) ?: return@mapNotNull null
            val ring = o.optJSONArray("ring") ?: JSONArray()
            TimerPlan.Entry(
                id = o.optString("id").ifEmpty { return@mapNotNull null },
                label = o.optString("label"),
                state = o.optString("state"),
                endsAt = o.optLong("ends"),
                remainingMs = o.optLong("left"),
                durationMs = o.optLong("dur"),
                ringOn = (0 until ring.length()).map { ring.optString(it) },
            )
        }
    }

    private fun saveEntries(ctx: Context, list: List<TimerPlan.Entry>) {
        val arr = JSONArray()
        list.forEach { e ->
            arr.put(
                JSONObject().put("id", e.id).put("label", e.label).put("state", e.state).put("ends", e.endsAt)
                    .put("left", e.remainingMs).put("dur", e.durationMs).put("ring", JSONArray(e.ringOn)),
            )
        }
        prefs(ctx).edit().putString(KEY_TIMERS, arr.toString()).apply()
    }

    private fun readMap(ctx: Context, key: String): MutableMap<String, String> {
        val o = runCatching { JSONObject(prefs(ctx).getString(key, "{}")!!) }.getOrNull() ?: return HashMap()
        return o.keys().asSequence().associateWith { o.optString(it) }.toMutableMap()
    }

    private fun writeMap(ctx: Context, key: String, m: Map<String, String>) {
        prefs(ctx).edit().putString(key, JSONObject(m).toString()).apply()
    }

    private fun entryOf(x: TimerItem) =
        TimerPlan.Entry(x.id, x.label, x.state, x.endsAt, x.remainingMs, x.durationMs, x.ringOn)

    // --- the plan ----------------------------------------------------------------

    /** The timers as they are now (every timer shown here) and this
     *  phone's node id: kept for the alarm, and looked at. Any thread. */
    fun sync(ctx: Context, timers: Collection<TimerItem>, me: String) {
        saveEntries(ctx, timers.map(::entryOf))
        if (me.isNotEmpty()) prefs(ctx).edit().putString(KEY_ME, me).apply()
        check(ctx)
    }

    /** Ring what ended here, quiet what's no longer ringing, show what
     *  runs, and set the alarm for the next end. */
    @Synchronized
    fun check(ctx: Context) {
        val list = entries(ctx)
        val me = prefs(ctx).getString(KEY_ME, "") ?: ""
        val rung = readMap(ctx, KEY_RUNG).mapNotNull { (k, v) -> v.toLongOrNull()?.let { k to it } }.toMap()
        val now = System.currentTimeMillis()
        val plan = TimerPlan.plan(list, me, rung, now)
        val nm = ctx.getSystemService(NotificationManager::class.java)
        plan.quiet.forEach { runCatching { nm?.cancel(alarmId(it)) } }
        plan.ring.forEach { id -> list.firstOrNull { it.id == id }?.let { ring(ctx, it) } }
        if (plan.rung != rung) writeMap(ctx, KEY_RUNG, plan.rung.mapValues { it.value.toString() })

        // The quiet notifications with the time left.
        val ongoing = TimerPlan.ongoing(list, now)
        val shown = prefs(ctx).getStringSet(KEY_SHOWN, emptySet()) ?: emptySet()
        val ids = ongoing.map { it.id }.toSet()
        shown.filter { it !in ids }.forEach { runCatching { nm?.cancel(runningId(it)) } }
        ongoing.forEach { showRunning(ctx, it, now) }
        prefs(ctx).edit().putStringSet(KEY_SHOWN, ids).apply()

        schedule(ctx, plan.next)
    }

    private fun alarmId(id: String) = "timer-ring|$id".hashCode()
    private fun runningId(id: String) = "timer|$id".hashCode()

    private fun schedule(ctx: Context, at: Long?) {
        val am = ctx.getSystemService(AlarmManager::class.java) ?: return
        val pi = PendingIntent.getBroadcast(
            ctx, 0,
            Intent(ctx, TimerAlarmReceiver::class.java).setAction(ACTION_ALARM),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
        if (at == null) {
            am.cancel(pi)
            return
        }
        runCatching {
            if (NoteReminders.exactAllowed(ctx)) am.setExactAndAllowWhileIdle(AlarmManager.RTC_WAKEUP, at, pi)
            else am.setAndAllowWhileIdle(AlarmManager.RTC_WAKEUP, at, pi)
        }.onFailure {
            // Exact taken away meanwhile: inexact still comes.
            runCatching { am.setAndAllowWhileIdle(AlarmManager.RTC_WAKEUP, at, pi) }
        }
    }

    private fun op(ctx: Context, id: String, op: String): PendingIntent {
        val i = Intent(ctx, TimerAlarmReceiver::class.java).setAction(ACTION_DO).putExtra(EXTRA_ID, id).putExtra(EXTRA_OP, op)
        return PendingIntent.getBroadcast(
            ctx, "timer|$op|$id".hashCode(), i,
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
    }

    private fun open(ctx: Context, code: Int): PendingIntent = PendingIntent.getActivity(
        ctx, code,
        Intent(ctx, MainActivity::class.java).putExtra(EXTRA_OPEN_TIMERS, true)
            .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_SINGLE_TOP),
        PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
    )

    private fun title(ctx: Context, e: TimerPlan.Entry) = e.label.ifBlank { ctx.getString(R.string.timer_untitled) }

    /** A running or paused timer, quietly (requires POST_NOTIFICATIONS on
     *  Android 13+; a no-op without). */
    private fun showRunning(ctx: Context, e: TimerPlan.Entry, now: Long) {
        createChannels(ctx)
        val paused = e.state == "paused"
        val b = NotificationCompat.Builder(ctx, CHANNEL_RUNNING)
            .setSmallIcon(android.R.drawable.ic_lock_idle_alarm)
            .setContentTitle(title(ctx, e))
            .setCategory(NotificationCompat.CATEGORY_STOPWATCH)
            .setOngoing(true)
            .setOnlyAlertOnce(true)
            .setSilent(true)
            .setContentIntent(open(ctx, runningId(e.id)))
        if (paused) {
            b.setContentText(ctx.getString(R.string.timer_paused_left, TimerPlan.clock(e.remainingMs)))
                .setShowWhen(false)
                .addAction(0, ctx.getString(R.string.timer_resume), op(ctx, e.id, "resume"))
        } else {
            b.setContentText(ctx.getString(R.string.timer_running_text))
                .setWhen(e.endsAt)
                .setShowWhen(true)
                .setUsesChronometer(true)
                .setChronometerCountDown(true)
                .setTimeoutAfter(maxOf(1L, e.endsAt - now))
                .addAction(0, ctx.getString(R.string.timer_pause), op(ctx, e.id, "pause"))
        }
        b.addAction(0, ctx.getString(R.string.timer_plus_minute), op(ctx, e.id, "add"))
            .addAction(0, ctx.getString(R.string.timer_cancel), op(ctx, e.id, "cancel"))
        runCatching { ctx.getSystemService(NotificationManager::class.java)?.notify(runningId(e.id), b.build()) }
    }

    /** It ended: ring (as an alarm) until Stop, +1 min, or it's stopped
     *  elsewhere. */
    private fun ring(ctx: Context, e: TimerPlan.Entry) {
        createChannels(ctx)
        runCatching { ctx.getSystemService(NotificationManager::class.java)?.cancel(runningId(e.id)) }
        val n = NotificationCompat.Builder(ctx, CHANNEL_ALARM)
            .setSmallIcon(android.R.drawable.ic_lock_idle_alarm)
            .setContentTitle(title(ctx, e))
            .setContentText(ctx.getString(R.string.timer_times_up))
            .setCategory(NotificationCompat.CATEGORY_ALARM)
            .setPriority(NotificationCompat.PRIORITY_MAX)
            .setVisibility(NotificationCompat.VISIBILITY_PUBLIC)
            .setWhen(e.endsAt)
            .setShowWhen(true)
            .setOngoing(true)
            .setTimeoutAfter(RING_FOR_MS)
            .setContentIntent(open(ctx, alarmId(e.id)))
            .addAction(0, ctx.getString(R.string.timer_stop), op(ctx, e.id, "dismiss"))
            .addAction(0, ctx.getString(R.string.timer_plus_minute), op(ctx, e.id, "add"))
            .build()
        // Rings over and over until it's dealt with (or times out).
        n.flags = n.flags or Notification.FLAG_INSISTENT
        runCatching { ctx.getSystemService(NotificationManager::class.java)?.notify(alarmId(e.id), n) }
    }

    // --- Stop, +1 min, Pause, Resume, Cancel from a notification ---------------

    /** Change the timer kept here at once (the notification follows, the
     *  alarm moves), and through the core now or once it runs. */
    fun change(ctx: Context, id: String, op: String) {
        val now = System.currentTimeMillis()
        synchronized(this) {
            saveEntries(ctx, entries(ctx).mapNotNull { if (it.id == id) TimerPlan.applied(it, op, now) else it })
        }
        check(ctx)
        synchronized(this) {
            val pending = readMap(ctx, KEY_PENDING)
            // Two +1 min taps before the core runs: both count.
            val before = pending[id]?.let { runCatching { JSONObject(it) }.getOrNull() }
            val adds = if (op == "add" && before?.optString("op") == "add") before.optInt("n", 1) + 1 else 1
            pending[id] = JSONObject().put("op", op).put("n", adds).toString()
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

    /** Do what was tapped on notifications while the core wasn't running.
     *  True if nothing is left. Off the main thread. */
    fun flushPending(ctx: Context): Boolean {
        val engine = Core.engine() ?: return false
        val pending = synchronized(this) { readMap(ctx, KEY_PENDING) }
        if (pending.isEmpty()) return true
        for ((id, json) in pending) {
            val o = runCatching { JSONObject(json) }.getOrNull()
            val op = o?.optString("op").orEmpty()
            val saved = runCatching {
                if (op == "add") engine.timerAdd(id, MINUTE * o!!.optInt("n", 1).coerceIn(1, 60))
                else engine.timerAction(id, op)
            }.getOrNull()
            // Gone meanwhile (stopped elsewhere): nothing more to do.
            if (saved != null) main.post { Timers.applySaved(saved) }
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

    /** Timers ring here when they end, but may ring late: Android hasn't
     *  let this app set exact alarms. */
    fun mayBeLate(ctx: Context) = !NoteReminders.exactAllowed(ctx)
}

/** The timer alarm, and a timer notification's buttons. */
class TimerAlarmReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        val ctx = context.applicationContext
        val id = intent.getStringExtra(TimerAlarms.EXTRA_ID)
        val op = intent.getStringExtra(TimerAlarms.EXTRA_OP)
        val pending = goAsync()
        Thread {
            try {
                when (intent.action) {
                    TimerAlarms.ACTION_ALARM -> TimerAlarms.check(ctx)
                    TimerAlarms.ACTION_DO -> if (id != null && op != null && op in OPS) TimerAlarms.change(ctx, id, op)
                }
            } finally {
                pending.finish()
            }
        }.start()
    }

    private companion object {
        val OPS = setOf("dismiss", "add", "pause", "resume", "cancel")
    }
}
