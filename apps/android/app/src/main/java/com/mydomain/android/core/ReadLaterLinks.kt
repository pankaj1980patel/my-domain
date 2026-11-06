package com.mydomain.android.core

/**
 * Read later's pure logic (no Android, no core: unit-tested on the JVM):
 * what a share holds ([fromShare]: the link and the title it came with)
 * and which saved links the screen lists ([visible]).
 *
 * The core checks a link again when it's saved (`features::read_later`:
 * http and https only); this side only picks it out of a share. Nothing is
 * fetched from the web: the title is the one the share gave (its subject,
 * or the words around the link), else the core uses the link's host.
 */
object ReadLaterLinks {
    /** The longest title the core keeps, in characters. */
    const val MAX_TITLE = 300

    private val URL = Regex("(?i)https?://[^\\s<>\"]+")
    private const val TRAILING = ".,;:!?'*"
    /** What a share puts between a title and its link ("Title - https://…"). */
    private const val SEPARATORS = "-–—|:·•"

    /** A link picked out of a share, and the title it came with (null:
     *  none; the core then uses the host). */
    data class Shared(val url: String, val title: String?)

    /** The first web link in a share's text (else its subject), and its
     *  title: the subject when it isn't itself a link, else the text's
     *  words around the link. Null when there's no web link. */
    fun fromShare(text: String?, subject: String?): Shared? {
        val body = text.orEmpty()
        val (url, from) = firstLink(body)?.let { it to body }
            ?: firstLink(subject.orEmpty())?.let { it to subject.orEmpty() }
            ?: return null
        val title = subject?.let(::tidy)?.takeIf { it.isNotEmpty() && URL.find(it) == null }
            ?: tidy(from.replace(url, " ")).trim { it.isWhitespace() || it in SEPARATORS || it in TRAILING }.takeIf { it.isNotEmpty() && URL.find(it) == null }
        return Shared(url, title?.takeIf { it.length <= MAX_TITLE })
    }

    /** The first web link in [text], without punctuation that follows it
     *  (a closing bracket only when the link didn't open one). */
    fun firstLink(text: String): String? {
        for (m in URL.findAll(text)) {
            var url = m.value
            while (true) {
                val before = url
                url = url.trimEnd { it in TRAILING }
                for ((open, close) in listOf('(' to ')', '[' to ']', '{' to '}')) {
                    if (url.endsWith(close) && url.count { it == close } > url.count { it == open }) url = url.dropLast(1)
                }
                if (url == before) break
            }
            if (hasHost(url)) return url
        }
        return null
    }

    private fun hasHost(url: String): Boolean {
        val rest = url.substringAfter("://", "")
        val authority = rest.split('/', '?', '#').first().substringAfterLast('@')
        return authority.substringBefore(':').isNotEmpty() || authority.startsWith("[")
    }

    /** One line, spaces run together. */
    fun tidy(s: String): String = s.split(Regex("\\s+")).filter { it.isNotEmpty() }.joinToString(" ")

    /** A saved link as the screen needs it (the core's ReadLaterEntry). */
    data class Item(val id: String, val url: String, val title: String, val host: String, val read: Boolean, val savedAt: Long)

    /** What the list shows: unread only unless [showRead], every typed word
     *  in the title, the host or the link; the most recently saved first. */
    fun visible(items: Collection<Item>, showRead: Boolean, query: String): List<Item> {
        val words = query.lowercase().split(Regex("\\s+")).filter { it.isNotEmpty() }
        return items
            .filter { showRead || !it.read }
            .filter { i ->
                val hay = "${i.title}\n${i.host}\n${i.url}".lowercase()
                words.all { hay.contains(it) }
            }
            .sortedWith(compareByDescending<Item> { it.savedAt }.thenBy { it.id })
    }
}
