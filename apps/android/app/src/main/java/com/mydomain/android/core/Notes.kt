package com.mydomain.android.core

import android.os.Handler
import android.os.Looper
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateMapOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import com.mydomain.android.ffi.NoteItem
import org.json.JSONObject

/**
 * Notes: the notepad that's the same on all your devices (the core's
 * `features::notes`; the desktop's `lib/notes.ts`). Every note by id: read
 * from the core at start ([refresh]), changed by what's saved here and by
 * `notes_changed` events ([applyChanged]: notes changed on your other
 * devices; nothing saved here comes back that way).
 *
 * A checklist is a convention over plain text: lines starting with "[ ] "
 * or "[x] " are drawn as boxes ([lines], [toggle], [addItem]).
 */
object Notes {
    /** The longest body the core keeps, in bytes. */
    const val MAX_BODY = 64 * 1024

    val byId = mutableStateMapOf<String, NoteItem>()
    var loaded by mutableStateOf(false); private set

    private val main = Handler(Looper.getMainLooper())

    /** Read every note from the core. Safe from any thread. */
    fun refresh() {
        val list = runCatching { Core.engine()?.notesList() }.getOrNull() ?: return
        main.post {
            byId.clear()
            list.forEach { byId[it.id] = it }
            loaded = true
        }
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
    )

    fun fields(n: NoteItem) = Fields(n.title, n.body, n.pinned, n.checklist, n.color)

    /** Save a note ([id] null: a new one; a deleted one comes back). Throws
     *  the core's reason when it can't be kept. Call from the main thread. */
    suspend fun save(id: String?, f: Fields): NoteItem {
        val saved = Core.call { it.noteSave(id, f.title, f.body, f.pinned, f.checklist, f.color) }
        byId[saved.id] = saved
        return saved
    }

    /** Save on the way out of the editor (the screen's scope is gone). */
    fun saveDetached(id: String?, f: Fields) {
        Thread {
            val saved = runCatching { Core.engine()?.noteSave(id, f.title, f.body, f.pinned, f.checklist, f.color) }.getOrNull()
            if (saved != null) main.post { byId[saved.id] = saved }
        }.start()
    }

    /** Delete a note on all your devices. Call from the main thread. */
    suspend fun delete(id: String) {
        Core.call { it.noteDelete(id) }
        byId.remove(id)
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
