package com.mydomain.android.core

import java.net.URLDecoder

/**
 * Join links (`mydomain://join?v=1&s=<server>&u=<user>&t=<secret>&k=<key>`,
 * made by mdcore's `join`) and the 6-digit codes beside them, as far as the
 * app needs to know before handing them to the core: is this one of ours,
 * whose account and which server does it join (to say so on screen), and
 * the digits of a typed code. Opening the key, and claiming the invite,
 * is the core's job. Pure Kotlin, so it is unit-tested on the JVM.
 */
object JoinLinks {
    const val SCHEME = "mydomain"
    const val HOST = "join"
    private const val PREFIX = "mydomain://join?"

    data class Link(val server: String, val username: String)

    fun isJoinLink(text: String): Boolean = text.trim().startsWith(PREFIX, ignoreCase = true)

    /** The account and server a link joins; null if it isn't a usable one
     *  (another version, a missing part, not http(s)). */
    fun parse(text: String): Link? {
        val t = text.trim()
        if (!isJoinLink(t)) return null
        val fields = HashMap<String, String>()
        for (pair in t.substring(PREFIX.length).split('&')) {
            val i = pair.indexOf('=')
            if (i <= 0) return null
            val value = runCatching { URLDecoder.decode(pair.substring(i + 1), "UTF-8") }.getOrNull() ?: return null
            fields[pair.substring(0, i)] = value
        }
        if (fields["v"] != "1") return null
        val server = fields["s"]?.takeIf { it.startsWith("https://") || it.startsWith("http://") } ?: return null
        val user = fields["u"]?.takeIf { it.isNotBlank() } ?: return null
        if (fields["t"].isNullOrEmpty() || fields["k"].isNullOrEmpty()) return null
        return Link(server, user)
    }

    /** The 6 digits of a typed code ("123 456", "123-456"), else null. */
    fun codeDigits(text: String): String? {
        val t = text.trim()
        if (t.any { !it.isDigit() && !it.isWhitespace() && it != '-' }) return null
        val digits = t.filter { it in '0'..'9' }
        return digits.takeIf { it.length == 6 }
    }

    /** "123456" -> "123 456", easier to read off a screen. */
    fun spaced(code: String): String = if (code.length == 6) code.substring(0, 3) + " " + code.substring(3) else code

    /** "4:59" for a countdown. */
    fun mmss(seconds: Int): String {
        val s = seconds.coerceAtLeast(0)
        return "${s / 60}:${(s % 60).toString().padStart(2, '0')}"
    }
}
