package com.mydomain.android.core

import android.os.Handler
import android.os.Looper
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateMapOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import com.mydomain.android.TimerAlarms
import com.mydomain.android.ffi.TimerItem
import org.json.JSONObject
import java.util.concurrent.Executors

/**
 * Timers: countdown timers shared between your devices (the core's
 * `features::timers`; the desktop's `lib/timers.ts`). Every timer shown,
 * by id: read from the core at start ([refresh]), changed by what's done
 * here and by `timers_changed` events ([applyChanged]: the whole list,
 * after a change on another device). Each counts down here from its end
 * time; nothing ticks over the network.
 *
 * Whenever the timers change here, they go to [TimerAlarms] (the
 * notification with the time left, and the alarm for the ones that ring
 * on this phone).
 */
object Timers {
    val byId = mutableStateMapOf<String, TimerItem>()
    var loaded by mutableStateOf(false); private set

    private val main = Handler(Looper.getMainLooper())
    /** Alarms are worked out off the main thread, one look at a time. */
    private val alarmThread = Executors.newSingleThreadExecutor()

    /** Read every timer from the core. Safe from any thread. Stop and +1
     *  min tapped on a notification while the core wasn't running go
     *  first. */
    fun refresh() {
        runCatching { TimerAlarms.flushPending(Core.app) }
        val list = runCatching { Core.engine()?.timersList() }.getOrNull() ?: return
        main.post {
            byId.clear()
            list.forEach { byId[it.id] = it }
            loaded = true
            alarms()
        }
    }

    /** The timers changed here: the notification and the alarm too. Main
     *  thread. */
    private fun alarms() {
        val all = byId.values.toList()
        val me = Core.me?.nodeId ?: ""
        val ctx = Core.app
        alarmThread.execute { runCatching { TimerAlarms.sync(ctx, all, me) } }
    }

    /** A timer changed outside the screen (a notification's Stop or +1
     *  min). Main thread. */
    fun applySaved(x: TimerItem) {
        if (x.state == "running" || x.state == "paused") byId[x.id] = x else byId.remove(x.id)
        alarms()
    }

    /** A `timers_changed` event: every timer shown, after a change on
     *  another device. Main thread. */
    fun applyChanged(o: JSONObject) {
        val arr = o.optJSONArray("timers") ?: return
        val list = (0 until arr.length()).mapNotNull { i -> arr.optJSONObject(i)?.let(::fromJson) }
        byId.clear()
        list.forEach { byId[it.id] = it }
        alarms()
    }

    private fun fromJson(o: JSONObject): TimerItem? {
        val id = o.optString("id").ifEmpty { null } ?: return null
        val ring = o.optJSONArray("ring_on")
        return TimerItem(
            id = id,
            label = o.optString("label"),
            durationMs = o.optLong("duration_ms"),
            state = o.optString("state"),
            endsAt = o.optLong("ends_at"),
            remainingMs = o.optLong("remaining_ms"),
            ringOn = if (ring == null) emptyList() else (0 until ring.length()).map { ring.optString(it) }.filter { it.isNotEmpty() },
            createdAt = o.optLong("created_at"),
            updatedAt = o.optLong("updated_at"),
            by = o.optString("by"),
            byName = o.optString("by_name"),
        )
    }

    /** Running ones by when they end (ended first), then paused ones. */
    fun sorted(): List<TimerItem> = byId.values.sortedWith(
        compareBy<TimerItem> { if (it.state == "running") 0 else 1 }.thenBy { it.endsAt }.thenBy { it.createdAt }.thenBy { it.id },
    )

    fun ended(x: TimerItem, now: Long) = TimerPlan.ended(x.state, x.endsAt, now)
    fun remaining(x: TimerItem, now: Long) = TimerPlan.remaining(x.state, x.endsAt, x.remainingMs, now)
    fun progress(x: TimerItem, now: Long) = TimerPlan.progress(x.state, x.endsAt, x.remainingMs, x.durationMs, now)

    /** Start a timer: it shows on your other devices and rings on
     *  [ringOn]. Throws the core's reason. Main thread. */
    suspend fun start(label: String, ms: Long, ringOn: List<String>): TimerItem {
        val x = Core.call { it.timerStart(label, ms, ringOn) }
        applySaved(x)
        return x
    }

    /** "pause", "resume", "cancel", "dismiss" (stop it ringing) or
     *  "restart", on every device. Main thread. */
    suspend fun action(id: String, action: String) {
        applySaved(Core.call { it.timerAction(id, action) })
    }

    /** Add time (an ended one starts again for that long). Main thread. */
    suspend fun add(id: String, ms: Long) {
        applySaved(Core.call { it.timerAdd(id, ms) })
    }

    /** Ring on these devices when it ends. Main thread. */
    suspend fun setRing(id: String, ringOn: List<String>) {
        applySaved(Core.call { it.timerSetRing(id, ringOn) })
    }

    /** Your other devices that have timers (node ids). Any thread. */
    fun devices(): List<String> {
        val e = Core.engine() ?: return emptyList()
        return Core.peers.map { it.nodeId }.filter { runCatching { e.timersSupported(it) }.getOrDefault(false) }
    }
}
