package com.mydomain.android.core

/*
 * Automation apps (Tasker, MacroDroid, Automate, ...) asking my-domain to
 * do things, the part with no Android in it: what a request says and
 * whether it makes sense, which device a name means, how often, the key's
 * check, and the history's JSON. Plain Kotlin, so the JVM unit tests run it
 * as is; the receivers (AutomationAppsReceiver.kt) feed it the intents.
 * The phone's counterpart of the desktop's `webhooks.rs`.
 */

/** One of your devices, as a request may name it: its [node] id, the name
 *  to show ([display]: the nickname, else its own name tidied) and every
 *  name it answers to ([names]: nickname, its own name, that tidied). */
data class AppPeer(val node: String, val display: String, val names: List<String>)

/** What an app asked for, checked. */
sealed class AppRequest {
    /** A device action ([args]: JSON object text). */
    data class Run(val device: String, val action: String, val args: String) : AppRequest()

    /** [text] onto a device's clipboard ([mode] "clipboard") or shown there
     *  as a notice ("notify"). */
    data class SendText(val device: String, val text: String, val mode: String) : AppRequest()

    /** One of this phone's automations, by id or label. */
    data class RunAutomation(val automation: String) : AppRequest()

    /** Your devices, as JSON. */
    data object ListDevices : AppRequest()
}

/** A request that can't be done; [message] says why, for people. */
class AppRefusal(message: String) : IllegalArgumentException(message)

/** One line of the history: at [ts] (ms), [caller] (a package name, or ""
 *  when Android didn't say) asked for [what]; [ok] and the [detail] (the
 *  transport, or why it failed). */
data class AppRun(val ts: Long, val caller: String, val what: String, val ok: Boolean, val detail: String)

object AutomationApps {
    /** The permission an app needs (and the user grants it) to use
     *  [AutomationAppsReceiver] without the key. */
    const val PERMISSION = "com.mydomain.android.permission.AUTOMATE"

    const val ACTION_RUN = "com.mydomain.android.action.RUN"
    const val ACTION_SEND_TEXT = "com.mydomain.android.action.SEND_TEXT"
    const val ACTION_RUN_AUTOMATION = "com.mydomain.android.action.RUN_AUTOMATION"
    const val ACTION_LIST_DEVICES = "com.mydomain.android.action.LIST_DEVICES"
    /** The answer, sent to `reply_package` when a request names one. */
    const val ACTION_RESULT = "com.mydomain.android.event.RESULT"

    const val EXTRA_DEVICE = "device"
    const val EXTRA_ACTION = "action"
    const val EXTRA_ARGS = "args"
    const val EXTRA_TEXT = "text"
    const val EXTRA_AS = "as"
    const val EXTRA_AUTOMATION = "automation"
    const val EXTRA_KEY = "key"
    const val EXTRA_REPLY_PACKAGE = "reply_package"
    /** Echoed back in the answer, so an app can tell its requests apart. */
    const val EXTRA_REQUEST_ID = "request_id"

    /** The key may come as the intent's data instead (Tasker's Send Intent
     *  has only three extras): `key:<the key>`. */
    const val KEY_SCHEME = "key"

    const val MAX_TEXT = 16 * 1024
    const val MAX_ARGS = 4 * 1024
    const val MAX_NAME = 200
    /** Things done (not lists) per [WINDOW_MS]: a loop in an automation app
     *  can't ring a computer all afternoon. */
    const val RUNS_PER_MINUTE = 6
    /** Device lists per [WINDOW_MS]. */
    const val LISTS_PER_MINUTE = 30
    /** Wrong keys per [WINDOW_MS]; then every keyed request is refused
     *  until the minute is up, so the key can't be guessed. */
    const val MAX_KEY_FAILURES = 10
    const val WINDOW_MS = 60_000L
    /** The history keeps this many, newest first. */
    const val HISTORY = 50

    /** The device actions anyone may ask for when the device hasn't said
     *  what it can do yet (the automations' presets); once it has, only
     *  what it listed, and "ring". */
    val KNOWN_ACTIONS = setOf("ring", "lock", "mute", "media", "keep_awake", "ringer", "flashlight")

    private val ACTION_NAME = Regex("^[a-z][a-z0-9_]{0,31}$")

    // --- requests ----------------------------------------------------------------

    /** The request in an intent with [action] and string extras [extra];
     *  throws [AppRefusal] with why it can't be. */
    fun parse(action: String?, extra: (String) -> String?): AppRequest = when (action) {
        ACTION_RUN -> {
            val device = name(extra(EXTRA_DEVICE), "device")
            val what = extra(EXTRA_ACTION)?.trim().orEmpty()
            if (what.isEmpty()) refuse("say which action, in the “action” extra")
            if (!ACTION_NAME.matches(what)) refuse("“$what” isn’t an action")
            AppRequest.Run(device, what, args(extra(EXTRA_ARGS)))
        }
        ACTION_SEND_TEXT -> {
            val device = name(extra(EXTRA_DEVICE), "device")
            val text = extra(EXTRA_TEXT).orEmpty()
            if (text.isBlank()) refuse("say what to send, in the “text” extra")
            if (text.length > MAX_TEXT) refuse("the text is longer than $MAX_TEXT characters")
            val mode = when (extra(EXTRA_AS)?.trim()?.lowercase().orEmpty()) {
                "", "clipboard" -> "clipboard"
                "notice", "notify" -> "notify"
                else -> refuse("“as” is clipboard or notice")
            }
            AppRequest.SendText(device, text, mode)
        }
        ACTION_RUN_AUTOMATION -> AppRequest.RunAutomation(name(extra(EXTRA_AUTOMATION), "automation"))
        ACTION_LIST_DEVICES -> AppRequest.ListDevices
        else -> refuse("my-domain doesn’t know “${action ?: ""}”")
    }

    private fun name(v: String?, what: String): String {
        val t = v?.trim().orEmpty()
        if (t.isEmpty()) refuse("say which $what, in the “$what” extra")
        if (t.length > MAX_NAME) refuse("that $what name is too long")
        return t
    }

    /** [raw] args as JSON object text ("{}" when none). */
    fun args(raw: String?): String {
        val t = raw?.trim().orEmpty()
        if (t.isEmpty()) return "{}"
        if (t.length > MAX_ARGS) refuse("“args” is longer than $MAX_ARGS characters")
        val v = runCatching { Json.parse(t) }.getOrNull()
        if (v !is Map<*, *>) refuse("“args” must be a JSON object, like {\"on\":true}")
        return t
    }

    /** Is [action] something to ask [device] for? [reported]: what it said
     *  it can do (null or empty: it hasn't said yet). Sending text has its
     *  own request. */
    fun checkAction(action: String, reported: Set<String>?, device: String) {
        if (action == "send_text") refuse("use the SEND_TEXT request to send text")
        val ok = action == "ring" ||
            if (reported.isNullOrEmpty()) action in KNOWN_ACTIONS else action in reported
        if (!ok) refuse("$device doesn’t take “$action”")
    }

    /** The node [query] means among [peers]: its node id, else a name it
     *  answers to (any case). None, or more than one, is an error. */
    fun findDevice(peers: List<AppPeer>, query: String): AppPeer {
        val q = query.trim()
        if (q.isEmpty()) refuse("say which device")
        peers.firstOrNull { it.node == q }?.let { return it }
        val named = peers.filter { p -> p.names.any { it.trim().equals(q, ignoreCase = true) } }
        return when (named.size) {
            1 -> named[0]
            0 -> refuse("there’s no device called “$q”")
            else -> refuse("more than one device is called “$q”; use its node id")
        }
    }

    /** The automation [query] means: its id, else its label (any case);
     *  it must be switched on. */
    fun findAutomation(list: List<Automation>, query: String): Automation {
        val q = query.trim()
        val a = list.firstOrNull { it.id == q } ?: run {
            val named = list.filter { it.label.isNotBlank() && it.label.trim().equals(q, ignoreCase = true) }
            when (named.size) {
                1 -> named[0]
                0 -> refuse("there’s no automation called “$q”")
                else -> refuse("more than one automation is called “$q”; use its id")
            }
        }
        if (!a.enabled) refuse("the automation “${a.label.ifBlank { a.id }}” is switched off")
        return a
    }

    /** The device list's JSON: `[{name, node, connected, actions}]`, as the
     *  desktop's `/v1/devices`. */
    fun devicesJson(peers: List<AppPeer>, live: Set<String>, actions: (String) -> Set<String>?): String =
        Json.write(
            peers.map { p ->
                linkedMapOf(
                    "name" to p.display,
                    "node" to p.node,
                    "connected" to (p.node in live),
                    "actions" to (listOf("ring") + (actions(p.node).orEmpty() - "ring").sorted()),
                )
            },
        )

    /** The answer's JSON: `{"ok": true, "via": ...}` or `{"ok": false, "error": ...}`. */
    fun resultJson(result: Result<String>): String = Json.write(
        result.fold(
            { linkedMapOf("ok" to true, "via" to it) },
            { linkedMapOf("ok" to false, "error" to (it.message ?: it.toString())) },
        ),
    )

    // --- the key -------------------------------------------------------------------

    /** The key in a request: the `key` extra, else data `key:<key>`. */
    fun keyOf(extra: String?, data: String?): String? =
        extra?.trim()?.takeIf { it.isNotEmpty() }
            ?: data?.trim()?.takeIf { it.startsWith("$KEY_SCHEME:", ignoreCase = true) }
                ?.substring(KEY_SCHEME.length + 1)?.trim()?.takeIf { it.isNotEmpty() }

    /** Is [given] the [expected] key? Constant time for keys of one length;
     *  no key set, or none given, is never a match. */
    fun keyMatches(expected: String?, given: String?): Boolean {
        if (expected.isNullOrEmpty() || given.isNullOrEmpty()) return false
        val a = expected.toByteArray(Charsets.UTF_8)
        val b = given.toByteArray(Charsets.UTF_8)
        var diff = a.size xor b.size
        for (i in a.indices) diff = diff or (a[i].toInt() xor b[i % b.size].toInt())
        return diff == 0
    }

    // --- the history -------------------------------------------------------------------

    /** [run] added to [list] (newest first), at most [HISTORY]. */
    fun record(list: List<AppRun>, run: AppRun): List<AppRun> = (listOf(run) + list).take(HISTORY)

    fun encodeHistory(list: List<AppRun>): String = Json.write(
        list.map { r ->
            linkedMapOf("ts" to r.ts, "caller" to r.caller, "what" to r.what, "ok" to r.ok, "detail" to r.detail)
        },
    )

    /** The history in [text]; unreadable text or entries are left out. */
    fun decodeHistory(text: String?): List<AppRun> {
        if (text.isNullOrBlank()) return emptyList()
        val items = runCatching { Json.parse(text) }.getOrNull() as? List<*> ?: return emptyList()
        return items.mapNotNull { item ->
            val o = item as? Map<*, *> ?: return@mapNotNull null
            AppRun(
                ts = (o["ts"] as? Number)?.toLong() ?: return@mapNotNull null,
                caller = o["caller"] as? String ?: "",
                what = o["what"] as? String ?: return@mapNotNull null,
                ok = o["ok"] as? Boolean ?: false,
                detail = o["detail"] as? String ?: "",
            )
        }.take(HISTORY)
    }

    private fun refuse(why: String): Nothing = throw AppRefusal(why)
}

/** At most [max] hits in any [spanMs]. Not thread-safe: the main thread
 *  uses it. */
class RateWindow(private val max: Int, private val spanMs: Long) {
    private val hits = ArrayDeque<Long>()

    /** Already [max] hits within the last [spanMs] before [now]? (A clock
     *  that went back forgets the ones "in the future".) */
    fun full(now: Long): Boolean {
        while (hits.isNotEmpty() && (now - hits.first() >= spanMs || hits.first() > now)) hits.removeFirst()
        return hits.size >= max
    }

    /** Count one, unless full. Was it counted? */
    fun hit(now: Long): Boolean {
        if (full(now)) return false
        hits.addLast(now)
        return true
    }
}
