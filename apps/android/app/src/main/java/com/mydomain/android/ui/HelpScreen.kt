package com.mydomain.android.ui

import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.heading
import android.content.Context
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.produceState
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.LinkAnnotation
import androidx.compose.ui.text.LinkInteractionListener
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.TextLinkStyles
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextDecoration
import androidx.compose.ui.unit.dp
import com.mydomain.android.R
import com.mydomain.android.core.HelpMarkdown
import com.mydomain.android.core.HelpTopic
import com.mydomain.android.core.MdBlock
import com.mydomain.android.core.MdInline
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

/** The phone's Help topics: the repo's docs/help, copied into the assets
 *  (assets/help) at build time, parsed once. */
object HelpTopics {
    @Volatile
    private var cache: List<HelpTopic>? = null

    /** This app's topics (android), in order; empty if none could be read. */
    fun load(ctx: Context): List<HelpTopic> = cache ?: run {
        val names = runCatching { ctx.assets.list("help") }.getOrNull().orEmpty()
        val all = names.filter { it.endsWith(".md") }.mapNotNull { name ->
            runCatching { ctx.assets.open("help/$name").bufferedReader().use { it.readText() } }
                .getOrNull()
                ?.let { HelpMarkdown.parseTopic(name, it) }
        }
        HelpMarkdown.topicsFor(all, "android").also { if (it.isNotEmpty()) cache = it }
    }
}

/**
 * Help (Settings → Help, or a "?" on a screen): search (every word, in a
 * topic's title, keywords or text) and the topic list; [topic] shows one
 * topic, whose links to other topics go through [onTopic].
 */
@Composable
fun HelpScreen(topic: String?, onTopic: (String?) -> Unit, onBack: () -> Unit) {
    val ctx = LocalContext.current
    val topics by produceState<List<HelpTopic>?>(null) {
        value = withContext(Dispatchers.IO) { HelpTopics.load(ctx) }
    }
    var query by rememberSaveable { mutableStateOf("") }
    val scroll = rememberScrollState()
    val all = topics
    val current = all?.firstOrNull { it.id == topic }
    // A new topic (or back to the list) starts at the top.
    LaunchedEffect(current?.id) { scroll.scrollTo(0) }

    Column(
        Modifier.fillMaxSize().verticalScroll(scroll).padding(start = 16.dp, end = 16.dp, bottom = 16.dp),
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        Row(Modifier.padding(top = 4.dp), verticalAlignment = Alignment.CenterVertically) {
            IconButton(onClick = { if (current != null) onTopic(null) else onBack() }) {
                LIcon(R.drawable.ic_lucide_arrow_left, T.text, 24.dp, stringResource(R.string.back))
            }
            Text(
                current?.title ?: stringResource(R.string.help_title), Modifier.semantics { heading() },
                style = MaterialTheme.typography.headlineSmall, color = T.text,
            )
        }
        when {
            all == null -> {}
            all.isEmpty() -> Muted(stringResource(R.string.help_missing))
            current != null -> Tile {
                HelpBlocks(current.blocks, all.map { it.id }.toSet()) { onTopic(it) }
            }
            else -> {
                Muted(stringResource(R.string.help_hint))
                Field(query, { query = it }, stringResource(R.string.help_search))
                val shown = HelpMarkdown.search(all, query)
                if (shown.isEmpty()) Muted(stringResource(R.string.help_none, query.trim()))
                shown.forEach { t ->
                    Tile(title = t.title, onClick = { onTopic(t.id) }) {
                        summary(t)?.let { Muted(it) }
                    }
                }
            }
        }
    }
}

/** The topic's first paragraph, cut short, as the list's second line. */
private fun summary(t: HelpTopic): String? {
    val first = t.blocks.firstOrNull { it is MdBlock.Para } as MdBlock.Para? ?: return null
    val text = HelpMarkdown.inlineText(first.inlines)
    return if (text.length <= 140) text else text.take(text.lastIndexOf(' ', 137).coerceAtLeast(100)) + "…"
}

@Composable
private fun HelpBlocks(blocks: List<MdBlock>, known: Set<String>, onTopic: (String) -> Unit) {
    val link = T.accentFg
    val code = T.surface2
    fun annotated(nodes: List<MdInline>): AnnotatedString =
        buildAnnotatedString { appendInlines(nodes, link, code, known, onTopic) }
    blocks.forEach { b ->
        when (b) {
            is MdBlock.Heading -> Text(
                annotated(b.inlines),
                Modifier.padding(top = 6.dp).semantics { heading() },
                style = if (b.level <= 2) MaterialTheme.typography.titleMedium else MaterialTheme.typography.titleSmall,
                color = T.text,
            )
            is MdBlock.Para -> Text(annotated(b.inlines), style = MaterialTheme.typography.bodyMedium, color = T.text)
            is MdBlock.ListBlock -> Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
                b.items.forEachIndexed { i, item ->
                    Row {
                        // TalkBack reads the item, not "bullet"; numbers stay.
                        Text(
                            if (b.ordered) "${i + 1}." else "•",
                            Modifier.width(22.dp).then(if (b.ordered) Modifier else Modifier.clearAndSetSemantics {}),
                            style = MaterialTheme.typography.bodyMedium, color = T.text2,
                        )
                        Text(annotated(item), Modifier.weight(1f), style = MaterialTheme.typography.bodyMedium, color = T.text)
                    }
                }
            }
        }
    }
}

/** Inline pieces into styled text; a link to a topic this app lists is
 *  tappable, one to a topic it doesn't stays plain, a web link shows its
 *  address (nothing is opened from Help). */
private fun AnnotatedString.Builder.appendInlines(
    nodes: List<MdInline>,
    link: Color,
    code: Color,
    known: Set<String>,
    onTopic: (String) -> Unit,
) {
    for (n in nodes) {
        when (n) {
            is MdInline.Text -> append(n.text)
            is MdInline.Code -> {
                pushStyle(SpanStyle(fontFamily = FontFamily.Monospace, background = code))
                append(n.text)
                pop()
            }
            is MdInline.Bold -> {
                pushStyle(SpanStyle(fontWeight = FontWeight.SemiBold))
                appendInlines(n.children, link, code, known, onTopic)
                pop()
            }
            is MdInline.Italic -> {
                pushStyle(SpanStyle(fontStyle = FontStyle.Italic))
                appendInlines(n.children, link, code, known, onTopic)
                pop()
            }
            is MdInline.Link -> {
                val href = HelpMarkdown.helpTarget(n.href)
                val target = href?.takeIf { it in known }
                if (target != null) {
                    pushLink(
                        LinkAnnotation.Clickable(
                            tag = target,
                            styles = TextLinkStyles(SpanStyle(color = link, textDecoration = TextDecoration.Underline)),
                            linkInteractionListener = LinkInteractionListener { onTopic(target) },
                        ),
                    )
                    appendInlines(n.children, link, code, known, onTopic)
                    pop()
                } else {
                    appendInlines(n.children, link, code, known, onTopic)
                    if (href == null) append(" (${n.href})")
                }
            }
        }
    }
}
