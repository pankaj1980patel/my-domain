package com.mydomain.android.core

/**
 * What the app holds right now that costs battery: wake locks, Wi-Fi and
 * multicast locks, foreground services, camera / audio / Bluetooth / GPS
 * use, listeners. Each place that takes one says [hold] and [release]; the
 * Developer screen lists them ("Resource ledger"), so a lock left behind
 * shows up as an entry that never goes away. Memory only, nothing logged.
 *
 * Pure (no Android) and thread-safe; [ResourceLedger] is the app's one book.
 */
class ResourceBook(private val clock: () -> Long = System::currentTimeMillis) {
    enum class Kind { WAKE_LOCK, WIFI_LOCK, MULTICAST_LOCK, SERVICE, CAMERA, AUDIO, BLUETOOTH, HOTSPOT, LOCATION, LISTENER }

    /** [until]: a lock with a timeout lets go by itself then (null: held until released). */
    data class Entry(val kind: Kind, val name: String, val since: Long, val until: Long?)

    private val held = LinkedHashMap<Pair<Kind, String>, Entry>()

    /** Taken (again: [timeoutMs] renews it, [since] stays). */
    fun hold(kind: Kind, name: String, timeoutMs: Long? = null) {
        val now = clock()
        synchronized(held) {
            val key = kind to name
            val since = held[key]?.since ?: now
            held[key] = Entry(kind, name, since, timeoutMs?.let { now + it })
        }
    }

    fun release(kind: Kind, name: String) {
        synchronized(held) { held.remove(kind to name) }
    }

    /** Held now, the oldest first (ones past their timeout are dropped). */
    fun snapshot(): List<Entry> {
        val now = clock()
        return synchronized(held) {
            held.values.removeAll { it.until != null && it.until <= now }
            held.values.sortedBy { it.since }
        }
    }

    fun isHeld(kind: Kind, name: String): Boolean = snapshot().any { it.kind == kind && it.name == name }

    /** "wake lock mydomain:push · 12 s · 18 s left" (the Developer screen). */
    fun describe(e: Entry): String {
        val now = clock()
        val age = ((now - e.since).coerceAtLeast(0)) / 1000
        val left = e.until?.let { " · ${((it - now).coerceAtLeast(0)) / 1000} s left" }.orEmpty()
        return "${e.kind.name.lowercase().replace('_', ' ')} ${e.name} · $age s$left"
    }
}

/** The app's ledger (see [ResourceBook]). */
object ResourceLedger {
    val book = ResourceBook()

    fun hold(kind: ResourceBook.Kind, name: String, timeoutMs: Long? = null) = book.hold(kind, name, timeoutMs)

    fun release(kind: ResourceBook.Kind, name: String) = book.release(kind, name)
}
