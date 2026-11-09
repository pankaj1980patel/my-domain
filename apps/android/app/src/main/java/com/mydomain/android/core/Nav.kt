package com.mydomain.android.core

/**
 * The app's back stack (the pure part, unit-tested): the screens pushed
 * over the tabs, oldest first. System Back pops the newest; an empty stack
 * shows the tabs. The app shell keeps it as saved text ([encode]), so it
 * survives a rotation and the process being recreated.
 */
data class Dest(val kind: Kind, val a: String? = null, val b: String? = null) {
    /** What each screen's [a] and [b] are. */
    enum class Kind {
        /** a: the node; b: what to bring up ("msg:<id>", "tab:<key>", …). */
        Device,
        Developer,

        /** a: an automation a search found. */
        Automations,

        /** "Set up this phone", the checklist. */
        Setup,
        Timers,

        /** a: a link a search found. */
        ReadLater,
        LinksMap,

        /** First-run setup. */
        Onboarding,
        Health,

        /** a: a note to open. */
        Notes,

        /** a: the tile to bring up (Search's Settings ids, "group:<name>"). */
        Settings,

        /** a: the tab's key; b: a transfer to bring up. */
        Files,

        /** a: "chats" or "notifications". */
        Messages,
        Search,

        /** a: the topic (none: the list). */
        Help,

        /** Notes, Timers and Read later. */
        Workspace,

        /** The links map and the health check. */
        ConnHealth,
    }
}

object NavStack {
    /** [d] on top (not twice in a row). */
    fun push(stack: List<Dest>, d: Dest): List<Dest> = if (stack.lastOrNull() == d) stack else stack + d

    /** Back: the newest one gone. */
    fun pop(stack: List<Dest>): List<Dest> = stack.dropLast(1)

    /** The newest one changed in place (its tab, what it found), so Back
     *  doesn't step through each change. */
    fun replaceTop(stack: List<Dest>, d: Dest): List<Dest> = if (stack.isEmpty()) listOf(d) else stack.dropLast(1) + d

    /** Leave out every screen of [kinds] (e.g. setup closing), keeping the rest in order. */
    fun without(stack: List<Dest>, vararg kinds: Dest.Kind): List<Dest> = stack.filter { it.kind !in kinds }

    /** One key per screen, for the fade between them: the same screen
     *  with other arguments (another tab, another help topic) is no change. */
    fun key(d: Dest): String = if (d.kind == Dest.Kind.Device) "Device:${d.a}" else d.kind.name

    // --- saved as text --------------------------------------------------------------

    private const val NULL = "\\0"

    private fun esc(s: String?): String =
        s?.replace("\\", "\\\\")?.replace("|", "\\p") ?: NULL

    private fun unesc(s: String): String? {
        if (s == NULL) return null
        val b = StringBuilder()
        var i = 0
        while (i < s.length) {
            val c = s[i]
            if (c == '\\' && i + 1 < s.length) {
                b.append(if (s[i + 1] == 'p') '|' else s[i + 1])
                i += 2
            } else {
                b.append(c)
                i++
            }
        }
        return b.toString()
    }

    /** Split on the '|' that aren't escaped. */
    private fun fields(line: String): List<String> {
        val out = mutableListOf<String>()
        val cur = StringBuilder()
        var i = 0
        while (i < line.length) {
            val c = line[i]
            when {
                c == '\\' && i + 1 < line.length -> { cur.append(c).append(line[i + 1]); i += 2 }
                c == '|' -> { out += cur.toString(); cur.clear(); i++ }
                else -> { cur.append(c); i++ }
            }
        }
        out += cur.toString()
        return out
    }

    /** Each screen as one line: kind|a|b. */
    fun encode(stack: List<Dest>): ArrayList<String> =
        ArrayList(stack.map { "${it.kind.name}|${esc(it.a)}|${esc(it.b)}" })

    /** [encode]'s lines back; a line that doesn't read (a screen this
     *  version doesn't have) is left out. */
    fun decode(lines: List<String>): List<Dest> = lines.mapNotNull { line ->
        val f = fields(line)
        if (f.size != 3) return@mapNotNull null
        val kind = Dest.Kind.entries.firstOrNull { it.name == f[0] } ?: return@mapNotNull null
        Dest(kind, unesc(f[1]), unesc(f[2]))
    }
}
