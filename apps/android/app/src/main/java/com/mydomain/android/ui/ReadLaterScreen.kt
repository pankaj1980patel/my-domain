package com.mydomain.android.ui

import android.content.Intent
import android.net.Uri
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.selection.selectableGroup
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.outlined.Info
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalClipboardManager
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.pluralStringResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import com.mydomain.android.R
import com.mydomain.android.core.ReadLater
import com.mydomain.android.core.ReadLaterLinks
import com.mydomain.android.core.friendlyError
import com.mydomain.android.core.timeAgo
import com.mydomain.android.ffi.ReadLaterEntry
import kotlinx.coroutines.launch

/**
 * Read later: links saved to read later, the same on all your devices
 * ([ReadLater]). Paste a link (or "Paste copied link"), tap one to open it
 * in the browser (which marks it read), mark it read or unread, or delete
 * it (with Undo). Unread by default; "All" shows the read ones too. Links
 * also come from the share sheet (ReadLaterShareActivity). Nothing is
 * fetched from the web: a link shows the title it was saved with, else its
 * host. [found]: a link a search result opened, brought into view.
 */
@Composable
fun ReadLaterScreen(onBack: () -> Unit, onHelp: () -> Unit, found: String? = null) {
    val ctx = LocalContext.current
    val toast = LocalToast.current
    val undo = LocalUndo.current
    val scope = rememberCoroutineScope()
    val clipboard = LocalClipboardManager.current
    var link by rememberSaveable { mutableStateOf("") }
    var title by rememberSaveable { mutableStateOf("") }
    var query by rememberSaveable { mutableStateOf("") }
    var showRead by rememberSaveable { mutableStateOf(found?.let { ReadLater.byId[it]?.read } ?: false) }
    val list = ReadLater.visible(showRead, query)
    val listState = rememberLazyListState()
    LaunchedEffect(found) {
        if (found != null && ReadLater.byId[found] != null) listState.scrollToKey(found)
    }

    val fail: (Throwable) -> Unit = { toast(friendlyError(it)) }
    val savedMsg = stringResource(R.string.read_later_saved)
    val deletedMsg = stringResource(R.string.read_later_deleted)
    val undoLabel = stringResource(R.string.read_later_undo)
    val noLink = stringResource(R.string.read_later_no_copied_link)
    val save = { text: String, t: String? ->
        scope.launch {
            try {
                val e = ReadLater.save(text, t)
                toast(String.format(savedMsg, e.title))
                link = ""
                title = ""
            } catch (e: Exception) {
                fail(e)
            }
        }
    }
    val open = { e: ReadLaterEntry ->
        runCatching {
            ctx.startActivity(Intent(Intent.ACTION_VIEW, Uri.parse(e.url)).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
        }.onFailure(fail).onSuccess {
            if (!e.read) scope.launch { runCatching { ReadLater.mark(e.id, true) }.onFailure(fail) }
        }
    }
    val delete = { e: ReadLaterEntry ->
        scope.launch {
            try {
                ReadLater.delete(e.id)
                undo(String.format(deletedMsg, e.title), undoLabel) {
                    scope.launch { runCatching { ReadLater.restore(e) }.onFailure(fail) }
                }
            } catch (err: Exception) {
                fail(err)
            }
        }
    }

    LazyColumn(
        Modifier.fillMaxSize(),
        state = listState,
        contentPadding = PaddingValues(start = 16.dp, end = 16.dp, bottom = 16.dp),
        verticalArrangement = Arrangement.spacedBy(10.dp),
    ) {
        item {
            Row(Modifier.padding(top = 4.dp), verticalAlignment = Alignment.CenterVertically) {
                IconButton(onClick = onBack) {
                    Icon(Icons.AutoMirrored.Filled.ArrowBack, stringResource(R.string.back), tint = T.text)
                }
                Text(
                    stringResource(R.string.read_later_title), Modifier.weight(1f).semantics { heading() },
                    style = MaterialTheme.typography.headlineSmall, color = T.text,
                )
                IconButton(onClick = onHelp) {
                    Icon(Icons.Outlined.Info, stringResource(R.string.help_open), tint = T.text2)
                }
            }
            Muted(stringResource(R.string.read_later_subtitle))
        }
        item {
            Tile {
                Field(link, { link = it }, stringResource(R.string.read_later_link_label))
                Field(title, { title = it.take(ReadLaterLinks.MAX_TITLE) }, stringResource(R.string.read_later_title_label))
                Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    PrimaryButton(stringResource(R.string.read_later_save), { save(link, title) }, enabled = link.isNotBlank())
                    SoftButton(stringResource(R.string.read_later_paste), {
                        val copied = clipboard.getText()?.text.orEmpty()
                        val shared = ReadLaterLinks.fromShare(copied, null)
                        if (shared == null) toast(noLink) else save(shared.url, shared.title)
                    })
                }
            }
        }
        item {
            Box(Modifier.fillMaxWidth()) { Field(query, { query = it }, stringResource(R.string.read_later_search)) }
        }
        item {
            Row(Modifier.selectableGroup(), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                val unread = stringResource(R.string.read_later_unread)
                val all = stringResource(R.string.read_later_all)
                if (!showRead) PrimaryButton(unread, { showRead = false }, selected = true) else SoftButton(unread, { showRead = false }, selected = false)
                if (showRead) PrimaryButton(all, { showRead = true }, selected = true) else SoftButton(all, { showRead = true }, selected = false)
            }
        }
        if (ReadLater.byId.isEmpty()) {
            item { EmptyState(stringResource(R.string.read_later_empty_title), stringResource(R.string.read_later_empty_body)) }
        } else if (list.isEmpty()) {
            item {
                Muted(
                    if (query.isNotBlank()) stringResource(R.string.read_later_no_match, query)
                    else stringResource(R.string.read_later_all_read),
                )
            }
        }
        items(list, key = { it.id }) { e ->
            LinkRow(e, Modifier.foundMark(e.id == found), onOpen = { open(e) }, onToggle = {
                scope.launch { runCatching { ReadLater.mark(e.id, !e.read) }.onFailure(fail) }
            }, onDelete = { delete(e) })
        }
    }
}

@Composable
private fun LinkRow(e: ReadLaterEntry, modifier: Modifier, onOpen: () -> Unit, onToggle: () -> Unit, onDelete: () -> Unit) {
    val meta = listOfNotNull(
        e.host.ifEmpty { null },
        e.savedOn.ifEmpty { null }?.let { stringResource(R.string.read_later_saved_on, it) },
        e.savedAt.takeIf { it > 0 }?.let { timeAgo(it) },
    ).joinToString(" · ")
    Tile(modifier) {
        Surface(onClick = onOpen, color = T.surface) {
            Column(Modifier.fillMaxWidth()) {
                Text(
                    e.title.ifEmpty { e.host }, style = MaterialTheme.typography.titleSmall,
                    fontWeight = if (e.read) FontWeight.Normal else FontWeight.SemiBold,
                    color = if (e.read) T.text2 else T.text, maxLines = 2, overflow = TextOverflow.Ellipsis,
                )
                Muted(meta)
            }
        }
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            GhostButton(
                stringResource(if (e.read) R.string.read_later_mark_unread else R.string.read_later_mark_read),
                onToggle, small = true,
            )
            GhostButton(stringResource(R.string.read_later_delete), onDelete, small = true)
        }
    }
}

/** Home's way in: "Read later" and how many are unread. */
@Composable
fun ReadLaterEntryCard(onOpen: () -> Unit) {
    val unread = ReadLater.unread()
    Tile(onClick = onOpen) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Text(stringResource(R.string.read_later_title), Modifier.weight(1f), style = MaterialTheme.typography.titleMedium, color = T.text)
            Muted(
                if (unread > 0) pluralStringResource(R.plurals.read_later_unread_count, unread, unread)
                else stringResource(R.string.read_later_none_unread),
            )
        }
    }
}
