package com.mydomain.android.core

/**
 * Help topics (the Markdown files in the repo's `docs/help`, copied into
 * the app's assets by the `copyHelp` Gradle task) as data. Pure (no
 * Android types), so the parser
 * and search are unit-tested; the screen is ui/HelpScreen.kt. The desktop
 * app has the same parser in `ui/src/help/markdown.ts` and `search.ts`.
 *
 * The Markdown subset: front matter (`key: value` between `---` lines),
 * `#`..`###` headings, paragraphs, one level of `-`/`*` or `1.` lists,
 * **bold**, *italic* or _italic_, `code` and [links](target). Anything else
 * is plain text.
 */
sealed class MdInline {
    data class Text(val text: String) : MdInline()
    data class Bold(val children: List<MdInline>) : MdInline()
    data class Italic(val children: List<MdInline>) : MdInline()
    data class Code(val text: String) : MdInline()
    data class Link(val href: String, val children: List<MdInline>) : MdInline()
}

sealed class MdBlock {
    data class Heading(val level: Int, val inlines: List<MdInline>) : MdBlock()
    data class Para(val inlines: List<MdInline>) : MdBlock()
    data class ListBlock(val ordered: Boolean, val items: List<List<MdInline>>) : MdBlock()
}

data class HelpTopic(
    /** From the file name: "04-find.md" → "find". Links use `help:find`. */
    val id: String,
    val title: String,
    val keywords: List<String>,
    val platforms: List<String>,
    val blocks: List<MdBlock>,
    /** Lower-case text, for search. */
    val body: String,
    /** Sort order: the file name's number. */
    val order: Int,
)

object HelpMarkdown {
    private val HEADING = Regex("""^(#{1,6})\s+(.*)$""")
    private val BULLET = Regex("""^\s*[-*+]\s+(.*)$""")
    private val NUMBERED = Regex("""^\s*\d+[.)]\s+(.*)$""")
    private val NUMBERED_NAME = Regex("""^(\d+)-(.+)$""")
    private const val ESCAPABLE = "\\`*_[]()#+-.!"

    /** A file split into its front matter (lower-case keys) and body. */
    fun splitFrontMatter(src: String): Pair<Map<String, String>, String> {
        val text = src.removePrefix("﻿").replace("\r\n", "\n").replace('\r', '\n')
        if (!text.startsWith("---\n")) return emptyMap<String, String>() to text
        val end = text.indexOf("\n---", 3)
        if (end < 0) return emptyMap<String, String>() to text
        val meta = LinkedHashMap<String, String>()
        for (line in text.substring(4, end).split('\n')) {
            val i = line.indexOf(':')
            if (i <= 0) continue
            meta[line.substring(0, i).trim().lowercase()] = line.substring(i + 1).trim()
        }
        val after = text.indexOf('\n', end + 4)
        return meta to (if (after < 0) "" else text.substring(after + 1))
    }

    /** "a, b ,c" → [a, b, c] (empty parts dropped). */
    fun splitList(value: String?): List<String> =
        (value ?: "").split(',').map { it.trim() }.filter { it.isNotEmpty() }

    /** Markdown body → blocks. */
    fun parseBlocks(body: String): List<MdBlock> {
        val out = mutableListOf<MdBlock>()
        val para = mutableListOf<String>()
        var listOrdered = false
        var list: MutableList<String>? = null

        fun flushPara() {
            if (para.isNotEmpty()) out += MdBlock.Para(parseInline(para.joinToString(" ")))
            para.clear()
        }
        fun flushList() {
            list?.let { items -> out += MdBlock.ListBlock(listOrdered, items.map { parseInline(it) }) }
            list = null
        }

        for (raw in body.replace("\r\n", "\n").replace('\r', '\n').split('\n')) {
            val line = raw.trimEnd()
            if (line.isBlank()) {
                flushPara(); flushList(); continue
            }
            val h = HEADING.find(line)
            if (h != null) {
                flushPara(); flushList()
                out += MdBlock.Heading(minOf(h.groupValues[1].length, 3), parseInline(h.groupValues[2].trim()))
                continue
            }
            val b = BULLET.find(line)
            val n = if (b == null) NUMBERED.find(line) else null
            if (b != null || n != null) {
                flushPara()
                val ordered = n != null
                if (list != null && listOrdered != ordered) flushList()
                if (list == null) {
                    list = mutableListOf()
                    listOrdered = ordered
                }
                list!!.add((b ?: n)!!.groupValues[1].trim())
                continue
            }
            val items = list
            if (items != null) {
                // An indented line carries on the item above; anything else ends the list.
                if (raw.first().isWhitespace()) {
                    items[items.lastIndex] = items.last() + " " + line.trim()
                    continue
                }
                flushList()
            }
            para += line.trim()
        }
        flushPara()
        flushList()
        return out
    }

    /** One line of text → inline pieces. Unclosed markers stay as text. */
    fun parseInline(src: String): List<MdInline> {
        val out = mutableListOf<MdInline>()
        val text = StringBuilder()
        fun push(node: MdInline) {
            if (text.isNotEmpty()) out += MdInline.Text(text.toString())
            text.setLength(0)
            out += node
        }
        fun wordAt(i: Int) = i in src.indices && (src[i].isLetterOrDigit() || src[i] == '_')
        var i = 0
        while (i < src.length) {
            val c = src[i]
            if (c == '\\' && i + 1 < src.length && src[i + 1] in ESCAPABLE) {
                text.append(src[i + 1]); i += 2; continue
            }
            if (c == '`') {
                val end = src.indexOf('`', i + 1)
                if (end > i + 1) {
                    push(MdInline.Code(src.substring(i + 1, end))); i = end + 1; continue
                }
            }
            if (src.startsWith("**", i) || src.startsWith("__", i)) {
                val mark = src.substring(i, i + 2)
                val end = src.indexOf(mark, i + 2)
                if (end > i + 2) {
                    push(MdInline.Bold(parseInline(src.substring(i + 2, end)))); i = end + 2; continue
                }
            }
            if ((c == '*' || c == '_') && i + 1 < src.length && src[i + 1] != ' ') {
                // `_` only at a word's edge, so snake_case words stay as they are.
                if (c == '*' || !wordAt(i - 1)) {
                    var end = src.indexOf(c, i + 1)
                    while (end > 0 && c == '_' && wordAt(end + 1)) end = src.indexOf(c, end + 1)
                    if (end > i + 1 && src[end - 1] != ' ') {
                        push(MdInline.Italic(parseInline(src.substring(i + 1, end)))); i = end + 1; continue
                    }
                }
            }
            if (c == '[') {
                val close = src.indexOf(']', i + 1)
                if (close > i && close + 1 < src.length && src[close + 1] == '(') {
                    val end = src.indexOf(')', close + 2)
                    if (end > close) {
                        push(MdInline.Link(src.substring(close + 2, end).trim(), parseInline(src.substring(i + 1, close))))
                        i = end + 1
                        continue
                    }
                }
            }
            text.append(c)
            i++
        }
        if (text.isNotEmpty()) out += MdInline.Text(text.toString())
        return out
    }

    /** The words of some inline pieces, as plain text. */
    fun inlineText(inlines: List<MdInline>): String = inlines.joinToString("") {
        when (it) {
            is MdInline.Text -> it.text
            is MdInline.Code -> it.text
            is MdInline.Bold -> inlineText(it.children)
            is MdInline.Italic -> inlineText(it.children)
            is MdInline.Link -> inlineText(it.children)
        }
    }

    /** All the text of some blocks, one block per line. */
    fun blocksText(blocks: List<MdBlock>): String = blocks.joinToString("\n") {
        when (it) {
            is MdBlock.Heading -> inlineText(it.inlines)
            is MdBlock.Para -> inlineText(it.inlines)
            is MdBlock.ListBlock -> it.items.joinToString("\n") { item -> inlineText(item) }
        }
    }

    /** The topic a `help:` link points to, or null for any other link. */
    fun helpTarget(href: String): String? =
        if (href.startsWith("help:")) href.removePrefix("help:").trim().ifEmpty { null } else null

    /** One file → a topic, or null when it has no title (e.g. a README). */
    fun parseTopic(fileName: String, src: String): HelpTopic? {
        val base = fileName.substringAfterLast('/').removeSuffix(".md").removeSuffix(".MD")
        val m = NUMBERED_NAME.find(base)
        val (meta, body) = splitFrontMatter(src)
        val title = meta["title"]?.takeIf { it.isNotEmpty() } ?: return null
        val blocks = parseBlocks(body)
        return HelpTopic(
            id = m?.groupValues?.get(2) ?: base,
            title = title,
            keywords = splitList(meta["keywords"]).map { it.lowercase() },
            platforms = splitList(meta["platforms"]).map { it.lowercase() },
            blocks = blocks,
            body = blocksText(blocks).lowercase(),
            order = m?.groupValues?.get(1)?.toIntOrNull() ?: Int.MAX_VALUE,
        )
    }

    /** Typographic and plain apostrophes match each other. */
    private fun fold(s: String) = s.lowercase().replace('’', '\'').replace('‘', '\'')

    /** The words of a query, lower-case. */
    fun queryWords(query: String): List<String> = fold(query).split(Regex("""\s+""")).filter { it.isNotEmpty() }

    /**
     * Topics with every word of the query somewhere in their title, keywords
     * or text, best first: a word in the title counts most, then in the
     * keywords, then in the text; ties keep the topics' own order. An empty
     * query keeps them all, in order.
     */
    fun search(topics: List<HelpTopic>, query: String): List<HelpTopic> {
        val words = queryWords(query)
        if (words.isEmpty()) return topics.sortedBy { it.order }
        val scored = mutableListOf<Pair<HelpTopic, Int>>()
        for (t in topics) {
            val title = fold(t.title)
            val keys = fold(t.keywords.joinToString(" "))
            val body = fold(t.body)
            var score = 0
            var all = true
            for (w in words) {
                val s = (if (title.contains(w)) 3 else 0) + (if (keys.contains(w)) 2 else 0) + (if (body.contains(w)) 1 else 0)
                if (s == 0) {
                    all = false
                    break
                }
                score += s
            }
            if (all) scored += t to score
        }
        return scored.sortedWith(compareByDescending<Pair<HelpTopic, Int>> { it.second }.thenBy { it.first.order }).map { it.first }
    }

    /** Topics listed on [platform] ("desktop" or "android"), in order. */
    fun topicsFor(topics: List<HelpTopic>, platform: String): List<HelpTopic> =
        topics.filter { platform in it.platforms }.sortedBy { it.order }
}
