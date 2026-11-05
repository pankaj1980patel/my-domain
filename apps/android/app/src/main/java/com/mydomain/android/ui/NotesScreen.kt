package com.mydomain.android.ui

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.background
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.selection.selectableGroup
import androidx.compose.foundation.selection.toggleable
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.outlined.Info
import androidx.compose.material3.Checkbox
import androidx.compose.material3.CheckboxDefaults
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.key
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableLongStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextDecoration
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import com.mydomain.android.R
import com.mydomain.android.core.DeviceLabels
import com.mydomain.android.core.Notes
import com.mydomain.android.core.friendlyError
import com.mydomain.android.core.niceName
import com.mydomain.android.core.timeAgo
import com.mydomain.android.ffi.NoteItem
import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withContext

/** How long after the last change a note saves itself. */
private const val SAVE_AFTER_MS = 600L

/** The open note when it's a new one (not saved yet). */
private const val NEW = "\u0000new"

/**
 * Notes (a bottom-bar tab): a small notepad that's the same on all your
 * devices ([Notes]). The list (search over titles and text, every word;
 * pinned first, then the most recently changed), and a note open in the
 * editor, which saves itself a moment after you stop typing. Delete has an
 * Undo on the snackbar. A note changed on two devices at once keeps both
 * texts: the one that lost is a separate note, marked as a conflict copy.
 */
@Composable
fun NotesScreen(onHelp: () -> Unit) {
    // The open note's id (NEW: a new one; null: the list). `opened` changes
    // only when you pick one, so a new note getting its id keeps the editor.
    var open by rememberSaveable { mutableStateOf<String?>(null) }
    var opened by rememberSaveable { mutableIntStateOf(0) }
    val show = { id: String? ->
        open = id ?: NEW
        opened++
    }
    val o = open
    if (o == null) {
        NotesList(onOpen = { show(it) }, onNew = { show(null) }, onHelp = onHelp)
    } else {
        key(opened) {
            NoteEditor(
                startId = o.takeIf { it != NEW },
                onClose = { open = null },
                onCreated = { open = it },
                onOpen = { show(it) },
            )
        }
    }
}

@Composable
private fun NotesList(onOpen: (String) -> Unit, onNew: () -> Unit, onHelp: () -> Unit) {
    var query by rememberSaveable { mutableStateOf("") }
    val all = Notes.sorted()
    val list = all.filter { Notes.matches(it, query) }
    val now = System.currentTimeMillis()

    LazyColumn(
        Modifier.fillMaxSize(),
        contentPadding = PaddingValues(16.dp),
        verticalArrangement = Arrangement.spacedBy(10.dp),
    ) {
        item {
            PageHead(stringResource(R.string.notes_title), stringResource(R.string.notes_subtitle)) {
                IconButton(onClick = onHelp) {
                    Icon(Icons.Outlined.Info, stringResource(R.string.help_open), tint = T.text2)
                }
            }
        }
        item {
            Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                Box(Modifier.weight(1f)) { Field(query, { query = it }, stringResource(R.string.notes_search)) }
                PrimaryButton(stringResource(R.string.notes_new), onNew)
            }
        }
        if (all.isEmpty()) {
            item { EmptyState(stringResource(R.string.notes_empty_title), stringResource(R.string.notes_empty_body)) }
        } else if (list.isEmpty()) {
            item { Muted(stringResource(R.string.notes_no_match, query)) }
        }
        items(list, key = { it.id }) { n -> NoteRow(n, now) { onOpen(n.id) } }
    }
}

/** One note in the list: colour mark, title, pinned, conflict, first line. */
@Composable
private fun NoteRow(n: NoteItem, now: Long, onOpen: () -> Unit) {
    val untitled = stringResource(R.string.notes_untitled)
    val snippet = if (n.checklist && Notes.progress(n.body).second > 0) {
        val (done, all) = Notes.progress(n.body)
        stringResource(R.string.notes_progress, done, all)
    } else {
        n.body.lineSequence().firstOrNull { it.isNotBlank() }?.take(120) ?: ""
    }
    Tile(onClick = onOpen) {
        Row(horizontalArrangement = Arrangement.spacedBy(10.dp)) {
            Box(Modifier.width(4.dp).height(40.dp).background(labelColor(n.color) ?: T.border, MaterialTheme.shapes.extraSmall))
            Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(2.dp)) {
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Text(
                        n.title.ifBlank { untitled }, Modifier.weight(1f),
                        style = MaterialTheme.typography.titleSmall, color = T.text,
                        maxLines = 1, overflow = TextOverflow.Ellipsis,
                    )
                    if (n.pinned) Text(stringResource(R.string.notes_pinned), style = MaterialTheme.typography.labelSmall, color = T.accentFg)
                }
                if (n.conflictOf != null) ConflictTag()
                if (snippet.isNotEmpty()) {
                    Text(snippet, style = MaterialTheme.typography.bodySmall, color = T.text2, maxLines = 1, overflow = TextOverflow.Ellipsis)
                }
                Text(timeAgo(n.updatedAt, now), style = MaterialTheme.typography.labelSmall, color = T.text3)
            }
        }
    }
}

@Composable
private fun ConflictTag() {
    Text(
        stringResource(R.string.notes_conflict_tag),
        Modifier.background(T.warning.copy(alpha = 0.25f), MaterialTheme.shapes.extraSmall).padding(horizontal = 6.dp, vertical = 1.dp),
        style = MaterialTheme.typography.labelSmall, color = T.text, fontWeight = FontWeight.Bold,
    )
}

@Composable
private fun NoteEditor(startId: String?, onClose: () -> Unit, onCreated: (String) -> Unit, onOpen: (String) -> Unit) {
    val toast = LocalToast.current
    val undo = LocalUndo.current
    val scope = rememberCoroutineScope()
    val start = remember { startId?.let { Notes.byId[it] } }
    var id by remember { mutableStateOf(startId) }
    var title by remember { mutableStateOf(start?.title ?: "") }
    var body by remember { mutableStateOf(start?.body ?: "") }
    var pinned by remember { mutableStateOf(start?.pinned ?: false) }
    var checklist by remember { mutableStateOf(start?.checklist ?: false) }
    var color by remember { mutableStateOf(start?.color) }
    var asText by remember { mutableStateOf(false) }
    var item by remember { mutableStateOf("") }
    var error by remember { mutableStateOf<String?>(null) }
    var saving by remember { mutableStateOf(false) }
    // Changed here and not saved yet; the version on screen; deleting.
    var dirty by remember { mutableStateOf(false) }
    var shown by remember { mutableLongStateOf(start?.updatedAt ?: 0L) }
    var deleting by remember { mutableStateOf(false) }
    val current = id?.let { Notes.byId[it] }
    val untitled = stringResource(R.string.notes_untitled)
    val elsewhere = stringResource(R.string.notes_deleted_elsewhere)
    val undoLabel = stringResource(R.string.notes_undo)
    val deletedFmt = stringResource(R.string.notes_deleted)
    val saveLock = remember { Mutex() }

    fun fields() = Notes.Fields(title, body, pinned, checklist, color)
    fun edit(block: () -> Unit) {
        block()
        dirty = true
    }

    // A moment after the last change: save (a new note with nothing in it
    // isn't kept).
    LaunchedEffect(title, body, pinned, checklist, color, dirty) {
        if (!dirty) return@LaunchedEffect
        delay(SAVE_AFTER_MS)
        val f = fields()
        if ((id == null && f.title.isBlank() && f.body.isBlank()) || Notes.tooLong(f.body)) return@LaunchedEffect
        dirty = false
        saving = true
        // Typing on cancels this effect: the save itself still finishes
        // (one at a time), so a new note gets its id once, not twice.
        withContext(NonCancellable) {
            saveLock.withLock {
                try {
                    val saved = Notes.save(id, f)
                    shown = saved.updatedAt
                    error = null
                    if (id == null) {
                        id = saved.id
                        onCreated(saved.id)
                    }
                } catch (e: Exception) {
                    error = friendlyError(e)
                } finally {
                    saving = false
                }
            }
        }
    }
    // Leaving with something typed: save it now.
    DisposableEffect(Unit) {
        onDispose {
            val f = fields()
            if (dirty && !deleting && !Notes.tooLong(f.body) && (id != null || f.title.isNotBlank() || f.body.isNotBlank())) {
                Notes.saveDetached(id, f)
            }
        }
    }
    // Changed on another device: show it, unless you're typing here.
    LaunchedEffect(current?.updatedAt) {
        val n = current ?: return@LaunchedEffect
        if (dirty || n.updatedAt == shown) return@LaunchedEffect
        shown = n.updatedAt
        title = n.title
        body = n.body
        pinned = n.pinned
        checklist = n.checklist
        color = n.color
    }
    // Deleted on another device while open (nothing typed here since).
    LaunchedEffect(current == null) {
        if (id != null && current == null && Notes.loaded && !deleting && !dirty) {
            toast(elsewhere)
            onClose()
        }
    }
    BackHandler { onClose() }

    Column(
        Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(16.dp),
        verticalArrangement = Arrangement.spacedBy(10.dp),
    ) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            IconButton(onClick = onClose) {
                Icon(Icons.AutoMirrored.Filled.ArrowBack, stringResource(R.string.back), tint = T.text)
            }
            Text(
                stringResource(R.string.notes_title), Modifier.weight(1f).semantics { heading() },
                style = MaterialTheme.typography.headlineSmall, color = T.text,
            )
            val n = current
            if (n != null) {
                GhostButton(stringResource(R.string.notes_delete), {
                    // What's on screen is what Undo brings back.
                    val last = Notes.fields(n).copy(title = title, body = body, pinned = pinned, checklist = checklist, color = color)
                    val msg = String.format(deletedFmt, title.ifBlank { untitled })
                    deleting = true
                    dirty = false
                    scope.launch {
                        try {
                            Notes.delete(n.id)
                        } catch (e: Exception) {
                            deleting = false
                            toast(friendlyError(e))
                            return@launch
                        }
                        // The snackbar outlives this screen; Undo saves it back.
                        undo(msg, undoLabel) { Notes.saveDetached(n.id, last) }
                        onClose()
                    }
                }, small = true)
            }
        }

        if (current?.conflictOf != null) {
            val name = niceName(current.byName.ifEmpty { stringResource(R.string.notes_another_device) })
            val original = current.conflictOf?.let { Notes.byId[it] }
            Column(
                Modifier.fillMaxWidth().background(T.warning.copy(alpha = 0.18f), MaterialTheme.shapes.small).padding(12.dp),
                verticalArrangement = Arrangement.spacedBy(6.dp),
            ) {
                ConflictTag()
                Text(stringResource(R.string.notes_conflict_body, name), style = MaterialTheme.typography.bodySmall, color = T.text)
                if (original != null) {
                    GhostButton(stringResource(R.string.notes_open_original, original.title.ifBlank { untitled }), { onOpen(original.id) }, small = true)
                }
            }
        }

        Field(title, { edit { title = it.take(200) } }, stringResource(R.string.notes_title_label))

        Row(
            Modifier.fillMaxWidth().horizontalScroll(rememberScrollState()),
            horizontalArrangement = Arrangement.spacedBy(8.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            SoftButton(
                stringResource(if (pinned) R.string.notes_unpin else R.string.notes_pin),
                { edit { pinned = !pinned } }, selected = pinned,
            )
            SoftButton(
                stringResource(R.string.notes_checklist),
                {
                    edit { checklist = !checklist }
                    asText = false
                },
                selected = checklist,
            )
        }

        Text(stringResource(R.string.notes_color), style = MaterialTheme.typography.labelLarge, color = T.text2)
        Row(Modifier.selectableGroup().horizontalScroll(rememberScrollState()), horizontalArrangement = Arrangement.spacedBy(6.dp)) {
            Swatch(null, color == null, stringResource(R.string.notes_no_color)) { edit { color = null } }
            DeviceLabels.COLORS.forEach { c ->
                Swatch(labelColor(c), color == c, colorName(c)) { edit { color = c } }
            }
        }

        if (checklist && !asText) {
            Notes.lines(body).forEachIndexed { i, l ->
                if (l.box) {
                    Row(
                        Modifier.fillMaxWidth().heightIn(min = 48.dp)
                            .toggleable(value = l.checked, role = Role.Checkbox, onValueChange = { edit { body = Notes.toggle(body, i) } }),
                        verticalAlignment = Alignment.CenterVertically,
                        horizontalArrangement = Arrangement.spacedBy(8.dp),
                    ) {
                        Checkbox(
                            checked = l.checked, onCheckedChange = null,
                            colors = CheckboxDefaults.colors(checkedColor = T.accent, uncheckedColor = T.controlBorder),
                        )
                        Text(
                            l.text, style = MaterialTheme.typography.bodyMedium,
                            color = if (l.checked) T.text3 else T.text,
                            textDecoration = if (l.checked) TextDecoration.LineThrough else null,
                        )
                    }
                } else {
                    Text(l.text, style = MaterialTheme.typography.bodyMedium, color = T.text)
                }
            }
            Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                Box(Modifier.weight(1f)) { Field(item, { item = it }, stringResource(R.string.notes_add_item)) }
                SoftButton(stringResource(R.string.notes_add), {
                    if (item.isNotBlank()) {
                        edit { body = Notes.addItem(body, item) }
                        item = ""
                    }
                })
            }
        } else {
            Field(body, { edit { body = it } }, stringResource(R.string.notes_body_label), singleLine = false)
        }
        if (checklist) {
            GhostButton(stringResource(if (asText) R.string.notes_as_list else R.string.notes_as_text), { asText = !asText }, small = true)
            if (asText) Muted(stringResource(R.string.notes_checklist_hint))
        }
        if (Notes.tooLong(body)) ErrorText(stringResource(R.string.notes_too_long))
        error?.let { ErrorText(it) }
        Muted(
            when {
                saving -> stringResource(R.string.notes_saving)
                current == null -> stringResource(R.string.notes_not_saved)
                else -> stringResource(
                    R.string.notes_saved_by,
                    niceName(current.byName.ifEmpty { stringResource(R.string.notes_another_device) }),
                    timeAgo(current.updatedAt),
                )
            },
        )
    }
}
