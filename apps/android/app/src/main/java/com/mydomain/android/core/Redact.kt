package com.mydomain.android.core

/**
 * Takes secrets out of a core event before it's kept for the Developer
 * screen or the debug event log ([Core.devLog], [Core.labLog]): one-time
 * codes, passwords, keys and tokens, and what may hold them (clipboard
 * text, a text sent marked as a password, codes in a notification).
 *
 * Works on the JSON text itself (no parser), so a payload that isn't valid
 * JSON is still cleaned; a value is replaced by [HIDDEN], its key kept, so
 * the log still says what happened.
 */
object Redact {
    const val HIDDEN = "‹hidden›"

    /** Keys whose value is a secret wherever they appear. */
    private val SECRET_KEYS = listOf(
        "code", "otp", "password", "pass", "passphrase", "psk", "pin", "secret",
        "token", "key", "private_key", "session_key", "auth", "cookie",
    )

    /** Events whose text is a code, a clipboard or a message: their text
     *  fields go too. */
    private val TEXT_EVENTS = setOf(
        "code-copied", "verification-code", "verification-code-gone",
        "clipboard-event", "clipboard-history",
    )

    /** Text fields of an event that may carry a secret. */
    private val TEXT_KEYS = listOf("text", "content", "body", "value", "title")

    private fun stringOrNumber() = """("(?:[^"\\]|\\.)*"|-?\d+(?:\.\d+)?)"""

    private fun keyed(keys: List<String>) =
        Regex(""""(${keys.joinToString("|") { Regex.escape(it) }})"\s*:\s*${stringOrNumber()}""", RegexOption.IGNORE_CASE)

    private val secretRe = keyed(SECRET_KEYS)
    private val textRe = keyed(TEXT_KEYS)
    /** A run of 4 to 10 digits on its own: what a one-time code looks like. */
    private val digitsRe = Regex("""(?<![\d.])\d{4,10}(?![\d.])""")
    /** A clipboard sync message inside a chat message (escaped JSON). */
    private val clipboardMsgRe = Regex("""\\?"type\\?"\s*:\s*\\?"clipboard""")
    private val sensitiveRe = Regex(""""sensitive"\s*:\s*true""")

    /** [payload] of event [name] with its secrets replaced by [HIDDEN]. */
    fun event(name: String, payload: String): String {
        var out = secretRe.replace(payload) { m -> "\"${m.groupValues[1]}\":\"$HIDDEN\"" }
        val allText = name in TEXT_EVENTS ||
            (name == "text-shared" && sensitiveRe.containsMatchIn(payload)) ||
            (name == "message-received" && clipboardMsgRe.containsMatchIn(payload))
        out = if (allText) {
            textRe.replace(out) { m -> "\"${m.groupValues[1]}\":\"$HIDDEN\"" }
        } else if (name.startsWith("notification") || name == "message-received" || name == "text-shared") {
            // Shown text stays (it's what the log is for), but a code in it doesn't.
            textRe.replace(out) { m -> "\"${m.groupValues[1]}\":${digitsRe.replace(m.groupValues[2], HIDDEN)}" }
        } else {
            out
        }
        return out
    }
}
