package com.mydomain.android.core

import android.os.Handler
import android.os.Looper
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateMapOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import com.mydomain.android.ffi.ReadLaterEntry
import org.json.JSONObject

/**
 * Read later: links saved to read later, the same on all your devices (the
 * core's `features::read_later`; the desktop's `lib/readLater.ts`). Every
 * link by id: read from the core at start ([refresh]), changed by what's
 * saved here and by `read_later_changed` events ([applyChanged]: links
 * changed on your other devices; nothing saved here comes back that way).
 *
 * Nothing is fetched from the web: a link shows the title it was shared
 * with, else its host. What a share holds and what the list shows is
 * [ReadLaterLinks] (pure, unit-tested).
 */
object ReadLater {
    val byId = mutableStateMapOf<String, ReadLaterEntry>()
    var loaded by mutableStateOf(false); private set

    private val main = Handler(Looper.getMainLooper())

    /** Read every link from the core. Safe from any thread. */
    fun refresh() {
        val list = runCatching { Core.engine()?.readLaterList() }.getOrNull() ?: return
        main.post {
            byId.clear()
            list.forEach { byId[it.id] = it }
            loaded = true
        }
    }

    /** A `read_later_changed` event: links changed on your other devices,
     *  and the ids of ones deleted there. Main thread. */
    fun applyChanged(o: JSONObject) {
        val items = o.optJSONArray("items")
        if (items != null) {
            for (i in 0 until items.length()) {
                val e = items.optJSONObject(i)?.let(::fromJson) ?: continue
                byId[e.id] = e
            }
        }
        val removed = o.optJSONArray("removed")
        if (removed != null) {
            for (i in 0 until removed.length()) byId.remove(removed.optString(i))
        }
    }

    private fun fromJson(o: JSONObject): ReadLaterEntry? {
        val id = o.optString("id").ifEmpty { null } ?: return null
        return ReadLaterEntry(
            id = id,
            url = o.optString("url"),
            title = o.optString("title"),
            host = o.optString("host"),
            read = o.optBoolean("read"),
            savedAt = o.optLong("saved_at"),
            savedOn = o.optString("saved_on"),
            updatedAt = o.optLong("updated_at"),
        )
    }

    fun item(e: ReadLaterEntry) = ReadLaterLinks.Item(e.id, e.url, e.title, e.host, e.read, e.savedAt)

    /** What the list shows (see [ReadLaterLinks.visible]). */
    fun visible(showRead: Boolean, query: String): List<ReadLaterEntry> =
        ReadLaterLinks.visible(byId.values.map(::item), showRead, query).mapNotNull { byId[it.id] }

    fun unread(): Int = byId.values.count { !it.read }

    /** Save a link ([text]: the link, or a share's text with one in it;
     *  [title] null or blank: its host). Throws the core's reason when it
     *  isn't a web link. Main thread. */
    suspend fun save(text: String, title: String?): ReadLaterEntry {
        val saved = Core.call { it.readLaterSave(text, title?.takeIf { t -> t.isNotBlank() }) }
        byId[saved.id] = saved
        return saved
    }

    /** Save from outside the screens (the share target), off the main thread. */
    fun saveNow(text: String, title: String?): ReadLaterEntry {
        val e = Core.engine() ?: throw IllegalStateException("not ready")
        val saved = e.readLaterSave(text, title?.takeIf { it.isNotBlank() })
        main.post { byId[saved.id] = saved }
        return saved
    }

    suspend fun mark(id: String, read: Boolean): ReadLaterEntry {
        val saved = Core.call { it.readLaterMark(id, read) }
        byId[saved.id] = saved
        return saved
    }

    suspend fun delete(id: String) {
        Core.call { it.readLaterDelete(id) }
        byId.remove(id)
    }

    /** Bring back a link just deleted (Undo), with its title and read mark. */
    suspend fun restore(e: ReadLaterEntry): ReadLaterEntry {
        val back = save(e.url, e.title)
        return if (e.read) mark(back.id, true) else back
    }
}
