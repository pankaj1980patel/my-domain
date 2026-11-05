package com.mydomain.android.core

import android.os.Handler
import android.os.Looper
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateMapOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import com.mydomain.android.NoteReminders
import com.mydomain.android.ffi.NoteItem
import org.json.JSONObject
import java.util.concurrent.Executors

/**
 * Notes: the notepad that's the same on all your devices (the core's
 * `features::notes`; the desktop's `lib/notes.ts`). Every note by id: read
 * from the core at start ([refresh]), changed by what's saved here and by
 * `notes_changed` events ([applyChanged]: notes changed on your other
 * devices; nothing saved here comes back that way).
 *
 * A checklist is a convention over plain text: lines starting with "[ ] "
 * or "[x] " are drawn as boxes ([lines], [toggle], [addItem]).
 *
 * Whenever the notes change here, their reminders go to [NoteReminders]
 * (which shows them, and takes down what's Done or given a new time).
 */
object Notes {
    /** The longest body the core keeps, in bytes. */
    const val MAX_BODY = 64 * 1024

    val byId = mutableStateMapOf<String, NoteItem>()
    var loaded by mutableStateOf(false); private set

    private val main = Handler(Looper.getMainLooper())
    /** Reminders are worked out off the main thread, one look at a time. */
    private val reminderThread = Executors.newSingleThreadExecutor()

    /** Read every note from the core. Safe from any thread. Done and Snooze
     *  tapped on a reminder while the core wasn't running are saved first. */
    fun refresh() {
        runCatching { NoteReminders.flushPending(Core.app) }
        val list = runCatching { Core.engine()?.notesList() }.getOrNull() ?: return
        main.post {
            byId.clear()
            list.forEach { byId[it.id] = it }
            loaded = true
            reminders()
        }
    }

    /** The notes changed here: their reminders too. Main thread. */
    private fun reminders() {
        val all = byId.values.toList()
        val ctx = Core.app
        reminderThread.execute { runCatching { NoteReminders.sync(ctx, all) } }
    }

    /** A note saved outside the screens (a reminder's Done or Snooze).
     *  Main thread. */
    fun applySaved(n: NoteItem) {
        byId[n.id] = n
        reminders()
    }

    /** A `notes_changed` event: notes changed on your other devices, and
     *  the ids of ones deleted there. Main thread. */
    fun applyChanged(o: JSONObject) {
        val notes = o.optJSONArray("notes")
        if (notes != null) {
            for (i in 0 until notes.length()) {
                val n = notes.optJSONObject(i)?.let(::fromJson) ?: continue
                byId[n.id] = n
            }
        }
        val removed = o.optJSONArray("removed")
        if (removed != null) {
            for (i in 0 until removed.length()) byId.remove(removed.optString(i))
        }
        reminders()
    }

    private fun JSONObject.text(key: String): String? =
        if (isNull(key)) null else optString(key).ifEmpty { null }

    private fun fromJson(o: JSONObject): NoteItem? {
        val id = o.optString("id").ifEmpty { null } ?: return null
        return NoteItem(
            id = id,
            title = o.optString("title"),
            body = o.optString("body"),
            pinned = o.optBoolean("pinned"),
            checklist = o.optBoolean("checklist"),
            color = o.text("color"),
            createdAt = o.optLong("created_at"),
            updatedAt = o.optLong("updated_at"),
            by = o.optString("by"),
            byName = o.optString("by_name"),
            conflictOf = o.text("conflict_of"),
            remindAt = if (o.isNull("remind_at")) null else o.optLong("remind_at"),
            remindDone = o.optBoolean("remind_done"),
        )
    }

    /** Pinned first, then the most recently changed. */
    fun sorted(): List<NoteItem> = byId.values.sortedWith(
        compareByDescending<NoteItem> { it.pinned }.thenByDescending { it.updatedAt }.thenBy { it.id },
    )

    /** Every typed word is in the title or the body. */
    fun matches(n: NoteItem, query: String): Boolean {
        val words = query.lowercase().split(Regex("\\s+")).filter { it.isNotEmpty() }
        if (words.isEmpty()) return true
        val hay = "${n.title}\n${n.body}".lowercase()
        return words.all { hay.contains(it) }
    }

    /** What the editor saves. */
    data class Fields(
        val title: String = "",
        val body: String = "",
        val pinned: Boolean = false,
        val checklist: Boolean = false,
        val color: String? = null,
        /** The reminder: when (unix ms; null: none), and marked Done. */
        val remindAt: Long? = null,
        val remindDone: Boolean = false,
    )

    fun fields(n: NoteItem) = Fields(n.title, n.body, n.pinned, n.checklist, n.color, n.remindAt, n.remindDone)

    /** Save a note ([id] null: a new one; a deleted one comes back). Throws
     *  the core's reason when it can't be kept. Call from the main thread. */
    suspend fun save(id: String?, f: Fields): NoteItem {
        val saved = Core.call { it.noteSave(id, f.title, f.body, f.pinned, f.checklist, f.color, f.remindAt, f.remindDone) }
        applySaved(saved)
        return saved
    }

    /** Save on the way out of the editor (the screen's scope is gone). */
    fun saveDetached(id: String?, f: Fields) {
        Thread {
            val saved = runCatching {
                Core.engine()?.noteSave(id, f.title, f.body, f.pinned, f.checklist, f.color, f.remindAt, f.remindDone)
            }.getOrNull()
            if (saved != null) main.post { applySaved(saved) }
        }.start()
    }

    /** Set note [id]'s reminder ([at] null: none; [done]: marked Done), the
     *  rest as the core has it. Throws the core's reason. Main thread. */
    suspend fun setReminder(id: String, at: Long?, done: Boolean): NoteItem {
        val saved = Core.call { e ->
            val n = e.noteGet(id) ?: throw IllegalStateException("no such note")
            e.noteSave(id, n.title, n.body, n.pinned, n.checklist, n.color, at, done)
        }
        applySaved(saved)
        return saved
    }

    /** A reminder still to come, or due and not Done. */
    fun pending(n: NoteItem) = n.remindAt != null && !n.remindDone

    /** Notes with a pending reminder, soonest first. */
    fun upcoming(): List<NoteItem> =
        byId.values.filter(::pending).sortedWith(compareBy<NoteItem> { it.remindAt }.thenBy { it.id })

    /** Delete a note on all your devices. Call from the main thread. */
    suspend fun delete(id: String) {
        Core.call { it.noteDelete(id) }
        byId.remove(id)
        reminders()
    }

    // --- checklists ----------------------------------------------------------

    data class Line(val box: Boolean, val checked: Boolean, val text: String)

    private val BOX = Regex("^\\[( |x|X)] (.*)$")

    fun lines(body: String): List<Line> =
        if (body.isEmpty()) emptyList()
        else body.split("\n").map { l ->
            val m = BOX.find(l)
            if (m != null) Line(true, m.groupValues[1] != " ", m.groupValues[2]) else Line(false, false, l)
        }

    /** [body] with box line [i] ticked or unticked. */
    fun toggle(body: String, i: Int): String {
        val lines = body.split("\n").toMutableList()
        val m = lines.getOrNull(i)?.let { BOX.find(it) } ?: return body
        lines[i] = "[${if (m.groupValues[1] == " ") "x" else " "}] ${m.groupValues[2]}"
        return lines.joinToString("\n")
    }

    /** [body] with a new unticked item at the end. */
    fun addItem(body: String, text: String): String {
        val item = "[ ] " + text.replace(Regex("\\s*\\n\\s*"), " ").trim()
        return if (body.isNotEmpty() && !body.endsWith("\n")) "$body\n$item" else body + item
    }

    /** Ticked and all boxes. */
    fun progress(body: String): Pair<Int, Int> {
        val boxes = lines(body).filter { it.box }
        return boxes.count { it.checked } to boxes.size
    }

    fun tooLong(body: String) = body.toByteArray(Charsets.UTF_8).size > MAX_BODY
}
