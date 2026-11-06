package com.mydomain.android.core

/*
 * The phone's automations, the part with no Android in it: what they are,
 * when one starts (trigger matching), at most once a minute (rate limit),
 * and how they're saved (JSON). Plain Kotlin, so the JVM unit tests run it
 * as is; `PhoneAutomations` (Automations.kt) feeds it what the phone sees
 * and does what it says. Mirrors the desktop's `automations.rs`.
 */

/** What can start an automation on this phone ([Trigger.kind]). */
object TriggerKind {
    /** A device (your computer, usually) gets a live link. */
    const val CONNECTS = "connects"
    /** That link has been down for a minute. */
    const val DISCONNECTS = "disconnects"
    /** This phone's battery goes from the level or above to below it. */
    const val BATTERY_BELOW = "battery_below"
    const val CHARGING_STARTS = "charging_starts"
    const val CHARGING_STOPS = "charging_stops"
    /** This phone joins Wi-Fi ([Trigger.ssid] "" = any network). */
    const val WIFI_JOINS = "wifi_joins"
    /** This phone leaves Wi-Fi ([Trigger.ssid] "" = any network). */
    const val WIFI_LEAVES = "wifi_leaves"

    val ALL = listOf(CONNECTS, DISCONNECTS, BATTERY_BELOW, CHARGING_STARTS, CHARGING_STOPS, WIFI_JOINS, WIFI_LEAVES)

    /** The kinds that watch another device (the rest watch this phone). */
    fun watchesDevice(kind: String) = kind == CONNECTS || kind == DISCONNECTS

    fun isWifi(kind: String) = kind == WIFI_JOINS || kind == WIFI_LEAVES
}

/** What starts an automation. [node]: the device watched (connects and
 *  disconnects only); [level]: for battery_below, 1-99; [ssid]: for the
 *  Wi-Fi kinds, the network's name, "" for any. */
data class Trigger(val kind: String, val node: String = "", val level: Int? = null, val ssid: String = "")

/** One automation: when [trigger], do [action] (with [args], JSON object
 *  text) on [node] ("" = this phone). [label] says what it does, for
 *  people; [lastFired] is when it last ran (ms since epoch). */
data class Automation(
    val id: String,
    val trigger: Trigger,
    val node: String,
    val action: String,
    val args: String = "{}",
    val label: String = "",
    val enabled: Boolean = true,
    val lastFired: Long? = null,
)

/** Something that happened, that an automation may be waiting for. */
sealed class Edge {
    data class Connected(val node: String) : Edge()
    /** The link has been down for [AutomationBook.AWAY_AFTER_MS]. */
    data class Disconnected(val node: String) : Edge()
    data class Battery(val from: Int, val to: Int) : Edge()
    data object ChargingStarted : Edge()
    data object ChargingStopped : Edge()
    /** Joined Wi-Fi called [ssid] (null: Android didn't say). [late]: the
     *  name of a network joined a moment ago, now known; only an automation
     *  for that name counts it (an "any Wi-Fi" one already ran). */
    data class WifiJoined(val ssid: String?, val late: Boolean = false) : Edge()
    data class WifiLeft(val ssid: String?) : Edge()
}

/**
 * The automations, and what's known about the phone to tell when something
 * changes. Not thread-safe: one thread (the main one) uses it.
 */
class AutomationBook(list: List<Automation> = emptyList()) {
    /** The automations, in the order they were added. */
    var list: List<Automation> = list
        private set

    private enum class LinkState { UNKNOWN, UP, DOWN, AWAY }
    private class Link(var state: LinkState = LinkState.UNKNOWN, var since: Long = 0)

    private val links = HashMap<String, Link>()
    private var battery: Int? = null
    private var charging: Boolean? = null
    private var wifiKnown = false
    private var wifiOn = false
    private var wifiSsid: String? = null
    /** A join was reported without its network's name. */
    private var joinedUnnamed = false
    /** Looks at Wi-Fi since then (the name may come late, or never). */
    private var unnamedLooks = 0

    // --- the list ------------------------------------------------------------

    /** Add [a] (checked); throws [IllegalArgumentException] with why not. */
    fun add(a: Automation): Automation {
        require(list.size < MAX_AUTOMATIONS) { "at most $MAX_AUTOMATIONS automations" }
        require(list.none { it.id == a.id }) { "that automation already exists" }
        val ok = checked(a)
        list = list + ok
        return ok
    }

    /** Change one (checked); when it last ran is kept. */
    fun update(a: Automation): Automation {
        val old = list.firstOrNull { it.id == a.id } ?: throw IllegalArgumentException("that automation is gone")
        val ok = checked(a).copy(lastFired = old.lastFired)
        list = list.map { if (it.id == a.id) ok else it }
        return ok
    }

    fun setEnabled(id: String, on: Boolean) {
        list = list.map { if (it.id == id) it.copy(enabled = on) else it }
    }

    fun delete(id: String): Boolean {
        val before = list.size
        list = list.filter { it.id != id }
        return list.size != before
    }

    // --- what the phone sees ---------------------------------------------------

    /** The live links now ([live], of the known [nodes]) at [now] (ms):
     *  "connects" when a link comes up for the first time or after counting
     *  as gone; "disconnects" once it's been down for a minute (a blip in
     *  between is neither). */
    fun links(nodes: Collection<String>, live: Set<String>, now: Long): List<Edge> =
        nodes.mapNotNull { node -> link(node, node in live, now) }

    fun link(node: String, up: Boolean, now: Long): Edge? {
        val l = links.getOrPut(node) { Link() }
        return when {
            up && (l.state == LinkState.UNKNOWN || l.state == LinkState.AWAY) -> {
                l.state = LinkState.UP
                Edge.Connected(node)
            }
            up -> {
                l.state = LinkState.UP
                null
            }
            l.state == LinkState.UP -> {
                l.state = LinkState.DOWN
                l.since = now
                null
            }
            l.state == LinkState.DOWN && now - l.since >= AWAY_AFTER_MS -> {
                l.state = LinkState.AWAY
                Edge.Disconnected(node)
            }
            else -> null
        }
    }

    /** A link went down and "disconnects" waits to see if it stays down,
     *  or Wi-Fi was joined and its name may still come: only then is a look
     *  on a timer needed (everything else comes from events). */
    fun waiting(): Boolean =
        links.values.any { it.state == LinkState.DOWN } || (joinedUnnamed && unnamedLooks < UNNAMED_LOOKS)

    /** When to look at the links again: soon while one [waiting], else
     *  rarely (a safety net for a missed event). */
    fun nextTickMs(): Long = if (waiting()) TICK_WAITING_MS else TICK_IDLE_MS

    /** This phone's battery is [pct] and [charging]: changes from the last
     *  values (none the first time, which only sets where things stand). */
    fun battery(pct: Int, charging: Boolean): List<Edge> {
        val out = ArrayList<Edge>()
        battery?.let { from -> if (from != pct) out += Edge.Battery(from, pct) }
        when (this.charging) {
            false -> if (charging) out += Edge.ChargingStarted
            true -> if (!charging) out += Edge.ChargingStopped
            null -> {}
        }
        battery = pct
        this.charging = charging
        return out
    }

    /** This phone is on Wi-Fi ([connected]) called [ssid] (null or blank:
     *  unknown, e.g. no location permission). The first look only sets
     *  where things stand. Moving from one named network to another is a
     *  leave and a join. */
    fun wifi(connected: Boolean, ssid: String?): List<Edge> {
        val name = ssid?.trim()?.takeIf { it.isNotEmpty() }
        if (!wifiKnown) {
            wifiKnown = true
            wifiOn = connected
            wifiSsid = if (connected) name else null
            return emptyList()
        }
        val out = ArrayList<Edge>()
        if (joinedUnnamed) unnamedLooks++
        when {
            !wifiOn && connected -> {
                out += Edge.WifiJoined(name)
                wifiSsid = name
                joinedUnnamed = name == null
                unnamedLooks = 0
            }
            wifiOn && !connected -> {
                out += Edge.WifiLeft(wifiSsid)
                wifiSsid = null
                joinedUnnamed = false
            }
            wifiOn && connected && name != null -> {
                val was = wifiSsid
                if (was == null) {
                    if (joinedUnnamed) out += Edge.WifiJoined(name, late = true)
                } else if (was != name) {
                    out += Edge.WifiLeft(was)
                    out += Edge.WifiJoined(name)
                }
                wifiSsid = name
                joinedUnnamed = false
            }
        }
        wifiOn = connected
        return out
    }

    // --- firing ----------------------------------------------------------------

    /** The automations [edge] starts at [now] (ms): on, matching, and not
     *  run in the last minute. They're marked as run (the caller saves). */
    fun take(edge: Edge, now: Long): List<Automation> {
        val out = ArrayList<Automation>()
        list = list.map { a ->
            if (a.enabled && matches(a.trigger, edge) && cooled(a.lastFired, now)) {
                val fired = a.copy(lastFired = now)
                out += fired
                fired
            } else a
        }
        return out
    }

    fun encode(): String = encode(list)

    companion object {
        /** How long a link must stay down to count as "disconnects". */
        const val AWAY_AFTER_MS = 60_000L
        /** The links' timer while a link is down (well under [AWAY_AFTER_MS]). */
        const val TICK_WAITING_MS = 15_000L
        /** ... and with none down: a rare look in case an event was missed. */
        const val TICK_IDLE_MS = 5 * 60_000L
        /** A Wi-Fi name that hasn't come after this many looks (about two
         *  minutes of quick ones) won't: the quick looks stop. */
        const val UNNAMED_LOOKS = 8
        /** The least time between two runs of one automation. */
        const val COOL_DOWN_MS = 60_000L
        const val MAX_AUTOMATIONS = 100
        const val MAX_LABEL = 80
        /** The longest Wi-Fi name (802.11). */
        const val MAX_SSID = 32
        /** The action that only posts a notification. */
        const val NOTIFY = "notify"

        /** Does [edge] start an automation with trigger [t]? Battery: only
         *  when it goes from the level or above to below it. */
        fun matches(t: Trigger, edge: Edge): Boolean = when (edge) {
            is Edge.Connected -> t.kind == TriggerKind.CONNECTS && t.node == edge.node
            is Edge.Disconnected -> t.kind == TriggerKind.DISCONNECTS && t.node == edge.node
            is Edge.Battery -> t.kind == TriggerKind.BATTERY_BELOW &&
                t.level != null && edge.from >= t.level && edge.to < t.level
            Edge.ChargingStarted -> t.kind == TriggerKind.CHARGING_STARTS
            Edge.ChargingStopped -> t.kind == TriggerKind.CHARGING_STOPS
            is Edge.WifiJoined -> t.kind == TriggerKind.WIFI_JOINS &&
                if (t.ssid.isEmpty()) !edge.late else edge.ssid.equals(t.ssid, ignoreCase = true)
            is Edge.WifiLeft -> t.kind == TriggerKind.WIFI_LEAVES &&
                (t.ssid.isEmpty() || edge.ssid.equals(t.ssid, ignoreCase = true))
        }

        /** May an automation that last ran at [last] run at [now]? (A clock
         *  that went back doesn't hold it up.) */
        fun cooled(last: Long?, now: Long): Boolean =
            last == null || now < last || now - last >= COOL_DOWN_MS

        /** [a], tidied; throws [IllegalArgumentException] if it can't be. */
        fun checked(a: Automation): Automation {
            val t = a.trigger
            require(t.kind in TriggerKind.ALL) { "not something that can start an automation" }
            val node = if (TriggerKind.watchesDevice(t.kind)) {
                t.node.trim().also { require(it.isNotEmpty()) { "choose a device to watch" } }
            } else ""
            val level = if (t.kind == TriggerKind.BATTERY_BELOW) {
                require(t.level != null && t.level in 1..99) { "the battery level is 1 to 99%" }
                t.level
            } else null
            val ssid = if (TriggerKind.isWifi(t.kind)) {
                t.ssid.trim().also { require(it.length <= MAX_SSID) { "a Wi-Fi name is at most $MAX_SSID characters" } }
            } else ""
            require(a.id.isNotBlank()) { "an automation needs an id" }
            require(a.action.isNotBlank()) { "choose what to do" }
            require(runCatching { Json.parse(a.args.ifBlank { "{}" }) }.getOrNull() is Map<*, *>) { "bad arguments" }
            return a.copy(
                trigger = Trigger(t.kind, node, level, ssid),
                node = a.node.trim(),
                args = a.args.ifBlank { "{}" },
                label = a.label.trim().take(MAX_LABEL),
            )
        }

        // --- saving ------------------------------------------------------------

        /** The file's text: `{"automations": [...]}`. */
        fun encode(list: List<Automation>): String = Json.write(
            mapOf(
                "automations" to list.map { a ->
                    linkedMapOf(
                        "id" to a.id,
                        "trigger" to linkedMapOf(
                            "kind" to a.trigger.kind,
                            "node" to a.trigger.node,
                            "level" to a.trigger.level,
                            "ssid" to a.trigger.ssid,
                        ),
                        "node" to a.node,
                        "action" to a.action,
                        "args" to a.args,
                        "label" to a.label,
                        "enabled" to a.enabled,
                        "last_fired" to a.lastFired,
                    )
                },
            ),
        )

        /** The automations in the file's [text]; ones that don't make sense
         *  (an unknown kind, say) are left out, and unreadable text is none. */
        fun decode(text: String): List<Automation> {
            val root = runCatching { Json.parse(text) }.getOrNull() as? Map<*, *> ?: return emptyList()
            val items = root["automations"] as? List<*> ?: return emptyList()
            return items.mapNotNull { item ->
                val o = item as? Map<*, *> ?: return@mapNotNull null
                val t = o["trigger"] as? Map<*, *> ?: return@mapNotNull null
                runCatching {
                    checked(
                        Automation(
                            id = o["id"] as? String ?: "",
                            trigger = Trigger(
                                kind = t["kind"] as? String ?: "",
                                node = t["node"] as? String ?: "",
                                level = (t["level"] as? Number)?.toInt(),
                                ssid = t["ssid"] as? String ?: "",
                            ),
                            node = o["node"] as? String ?: "",
                            action = o["action"] as? String ?: "",
                            args = o["args"] as? String ?: "{}",
                            label = o["label"] as? String ?: "",
                            enabled = o["enabled"] as? Boolean ?: true,
                            lastFired = (o["last_fired"] as? Number)?.toLong(),
                        ),
                    )
                }.getOrNull()
            }.distinctBy { it.id }.take(MAX_AUTOMATIONS)
        }
    }
}

/**
 * A small JSON reader and writer (org.json is Android's, and absent from
 * JVM unit tests). Objects read as `Map<String, Any?>` (in order), arrays as
 * `List<Any?>`, whole numbers as `Long`, others as `Double`.
 */
object Json {
    fun parse(text: String): Any? {
        val p = Parser(text)
        p.space()
        val v = p.value()
        p.space()
        if (p.i != text.length) throw IllegalArgumentException("trailing text at ${p.i}")
        return v
    }

    fun write(v: Any?): String = StringBuilder().also { write(it, v) }.toString()

    private fun write(sb: StringBuilder, v: Any?) {
        when (v) {
            null -> sb.append("null")
            is Boolean -> sb.append(v)
            is Int, is Long, is Short, is Byte -> sb.append(v)
            is Number -> {
                val d = v.toDouble()
                require(!d.isNaN() && !d.isInfinite()) { "not a JSON number" }
                if (d == Math.floor(d) && Math.abs(d) < 1e15) sb.append(d.toLong()) else sb.append(d)
            }
            is String -> quote(sb, v)
            is Map<*, *> -> {
                sb.append('{')
                var first = true
                for ((k, value) in v) {
                    if (!first) sb.append(',')
                    first = false
                    quote(sb, k.toString())
                    sb.append(':')
                    write(sb, value)
                }
                sb.append('}')
            }
            is Iterable<*> -> {
                sb.append('[')
                var first = true
                for (value in v) {
                    if (!first) sb.append(',')
                    first = false
                    write(sb, value)
                }
                sb.append(']')
            }
            else -> quote(sb, v.toString())
        }
    }

    private fun quote(sb: StringBuilder, s: String) {
        sb.append('"')
        for (c in s) {
            when (c) {
                '"' -> sb.append("\\\"")
                '\\' -> sb.append("\\\\")
                '\n' -> sb.append("\\n")
                '\r' -> sb.append("\\r")
                '\t' -> sb.append("\\t")
                '\b' -> sb.append("\\b")
                '\u000C' -> sb.append("\\f")
                else -> if (c < ' ') sb.append(String.format("\\u%04x", c.code)) else sb.append(c)
            }
        }
        sb.append('"')
    }

    private class Parser(val s: String) {
        var i = 0

        fun space() {
            while (i < s.length && s[i].isWhitespace()) i++
        }

        private fun fail(why: String): Nothing = throw IllegalArgumentException("$why at $i")

        private fun expect(c: Char) {
            if (i >= s.length || s[i] != c) fail("expected '$c'")
            i++
        }

        fun value(): Any? {
            if (i >= s.length) fail("unexpected end")
            return when (val c = s[i]) {
                '{' -> obj()
                '[' -> arr()
                '"' -> str()
                't' -> word("true", true)
                'f' -> word("false", false)
                'n' -> word("null", null)
                else -> if (c == '-' || c.isDigit()) num() else fail("unexpected '$c'")
            }
        }

        private fun word(w: String, v: Any?): Any? {
            if (!s.startsWith(w, i)) fail("expected $w")
            i += w.length
            return v
        }

        private fun obj(): Map<String, Any?> {
            expect('{')
            val m = LinkedHashMap<String, Any?>()
            space()
            if (i < s.length && s[i] == '}') { i++; return m }
            while (true) {
                space()
                val k = str()
                space()
                expect(':')
                space()
                m[k] = value()
                space()
                if (i < s.length && s[i] == ',') { i++; continue }
                expect('}')
                return m
            }
        }

        private fun arr(): List<Any?> {
            expect('[')
            val l = ArrayList<Any?>()
            space()
            if (i < s.length && s[i] == ']') { i++; return l }
            while (true) {
                space()
                l += value()
                space()
                if (i < s.length && s[i] == ',') { i++; continue }
                expect(']')
                return l
            }
        }

        private fun str(): String {
            expect('"')
            val sb = StringBuilder()
            while (true) {
                if (i >= s.length) fail("unterminated string")
                val c = s[i++]
                when {
                    c == '"' -> return sb.toString()
                    c == '\\' -> {
                        if (i >= s.length) fail("unterminated string")
                        when (val e = s[i++]) {
                            '"' -> sb.append('"')
                            '\\' -> sb.append('\\')
                            '/' -> sb.append('/')
                            'b' -> sb.append('\b')
                            'f' -> sb.append('\u000C')
                            'n' -> sb.append('\n')
                            'r' -> sb.append('\r')
                            't' -> sb.append('\t')
                            'u' -> {
                                if (i + 4 > s.length) fail("bad \\u escape")
                                val code = s.substring(i, i + 4).toIntOrNull(16) ?: fail("bad \\u escape")
                                sb.append(code.toChar())
                                i += 4
                            }
                            else -> fail("bad escape '\\$e'")
                        }
                    }
                    else -> sb.append(c)
                }
            }
        }

        private fun num(): Any {
            val start = i
            if (s[i] == '-') i++
            while (i < s.length && (s[i].isDigit() || s[i] in ".eE+-")) i++
            val t = s.substring(start, i)
            if (t.none { it in ".eE" }) t.toLongOrNull()?.let { return it }
            return t.toDoubleOrNull() ?: fail("bad number '$t'")
        }
    }
}
