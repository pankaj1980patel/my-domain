package com.mydomain.android.ui

import android.content.Context
import androidx.annotation.StringRes
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.border
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyListState
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.relocation.BringIntoViewRequester
import androidx.compose.foundation.relocation.bringIntoViewRequester
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.filled.Clear
import androidx.compose.material.icons.filled.Search
import androidx.compose.material3.FilterChip
import androidx.compose.material3.FilterChipDefaults
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.OutlinedTextFieldDefaults
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.runtime.snapshotFlow
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalFocusManager
import androidx.compose.ui.res.pluralStringResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.role
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import com.mydomain.android.AutomationAppsGate
import com.mydomain.android.PhoneAutomations
import com.mydomain.android.R
import com.mydomain.android.core.ClipLog
import com.mydomain.android.core.Core
import com.mydomain.android.core.Findable
import com.mydomain.android.core.Found
import com.mydomain.android.core.FoundGroup
import com.mydomain.android.core.FoundKind
import com.mydomain.android.core.HelpMarkdown
import com.mydomain.android.core.HelpTopic
import com.mydomain.android.core.Search
import com.mydomain.android.core.TransferLog
import com.mydomain.android.core.TransferState
import com.mydomain.android.core.niceName
import com.mydomain.android.core.timeAgo
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.withContext

/*
 * Search everything on the phone: one query over what my-domain keeps here
 * (devices, this session's messages, the clipboard history, transfers,
 * synced folders, automations, the automation apps' history, Settings and
 * Help), with the desktop's rules (core/Search.kt). Read through each
 * store's own lists; nothing is indexed or saved, and nothing leaves the
 * phone.
 */

/** The groups' names, for headings and the type chips. */
@StringRes
fun kindLabel(kind: FoundKind): Int = when (kind) {
    FoundKind.DEVICES -> R.string.search_kind_devices
    FoundKind.MESSAGES -> R.string.search_kind_messages
    FoundKind.CLIPBOARD -> R.string.search_kind_clipboard
    FoundKind.TRANSFERS -> R.string.search_kind_transfers
    FoundKind.FOLDERS -> R.string.search_kind_folders
    FoundKind.AUTOMATIONS -> R.string.search_kind_automations
    FoundKind.AUTOMATION_APPS -> R.string.search_kind_automation_apps
    FoundKind.SETTINGS -> R.string.search_kind_settings
    FoundKind.HELP -> R.string.search_kind_help
}

/** A Settings tile as searched: its id (SettingsScreen's `focus`), its
 *  title and the words on it. */
private class SettingsSection(val id: String, @StringRes val title: Int, @StringRes val words: List<Int>)

/** The Settings tiles, in their order on the screen. */
private val SETTINGS_SECTIONS = listOf(
    SettingsSection("help", R.string.help_title, listOf(R.string.help_settings_hint)),
    SettingsSection("account", R.string.settings_account, listOf(R.string.settings_sign_out)),
    SettingsSection(
        "device", R.string.settings_this_device,
        listOf(
            R.string.settings_clip_sync, R.string.settings_clip_history, R.string.settings_clip_history_hint,
            R.string.settings_share_codes, R.string.settings_share_codes_hint,
            R.string.settings_share_labels, R.string.settings_share_labels_hint,
        ),
    ),
    SettingsSection("automations", R.string.automations_title, listOf(R.string.automations_settings_hint)),
    SettingsSection("automation-apps", R.string.automation_apps_title, listOf(R.string.automation_apps_hint)),
    SettingsSection(
        "photos", R.string.settings_photos,
        listOf(
            R.string.settings_photos_hint, R.string.settings_backup, R.string.settings_backup_hint,
            R.string.settings_battery_alerts, R.string.settings_battery_charged,
        ),
    ),
    SettingsSection("sync", R.string.sync_title, listOf(R.string.sync_hint)),
    SettingsSection("touchpad", R.string.touchpad_title, listOf(R.string.settings_touchpad_secure_hint)),
    SettingsSection("location", R.string.location_title, listOf(R.string.location_hint)),
    SettingsSection("helper", R.string.helper_title, listOf(R.string.helper_hint)),
    SettingsSection("shizuku", R.string.shizuku_title, listOf(R.string.shizuku_hint)),
    SettingsSection("key", R.string.settings_device_key, listOf(R.string.settings_key_hint)),
    SettingsSection("appearance", R.string.settings_appearance, listOf(R.string.settings_appearance_hint)),
)

/** Help topics and Settings tiles as searched (they don't change while the
 *  app runs, so they're built once). */
private fun staticFindables(ctx: Context, topics: List<HelpTopic>): List<Findable> {
    val settings = ctx.getString(R.string.tab_settings)
    val sections = SETTINGS_SECTIONS.map { s ->
        Findable(
            FoundKind.SETTINGS, s.id, ctx.getString(s.title), settings,
            s.words.joinToString(" ") { ctx.getString(it) },
        )
    }
    val help = topics.map { t ->
        // The text as written (HelpTopic.body is lower-cased), for the snippet.
        Findable(FoundKind.HELP, t.id, t.title, t.keywords.joinToString(" "), HelpMarkdown.blocksText(t.blocks))
    }
    return sections + help
}

/**
 * Everything the phone can find, read from each store's own list (copies;
 * no storage changes). Recomposes when a store changes.
 *
 * TODO(notes): once Notes are in the app, add them here as
 * `Findable(FoundKind.NOTES, note.id, note.title, body = note.text, ts = note.updated)`.
 */
@Composable
private fun rememberFindables(ctx: Context): List<Findable> {
    // Loaded once each (no-ops when already loaded), as their own screens do.
    remember { PhoneAutomations.load(ctx) }
    LaunchedEffect(Unit) { AutomationAppsGate.load(ctx) }
    val topics by produceState<List<HelpTopic>>(emptyList()) {
        value = withContext(Dispatchers.IO) { HelpTopics.load(ctx) }
    }
    val fixed = remember(topics) { staticFindables(ctx, topics) }
    val sent = stringResource(R.string.transfers_dir_sent)
    val received = stringResource(R.string.transfers_dir_received)
    val underWay = stringResource(R.string.transfers_active)
    val failed = stringResource(R.string.search_tag_failed)

    val peers = Core.peers
    val messages = Core.messages.toList()
    val clips = ClipLog.list
    val transfers = TransferLog.list
    val folders = Core.syncPairs.toList()
    val automations = PhoneAutomations.items.toList()
    val runs = AutomationAppsGate.history.toList()
    // Names can change (a nickname): part of the key.
    val names = peers.map { Core.peerName(it.nodeId) }

    return remember(peers, names, messages, clips, transfers, folders, automations, runs, fixed) {
        buildList {
            peers.forEachIndexed { i, p ->
                add(Findable(FoundKind.DEVICES, p.nodeId, names[i], niceName(p.name), node = p.nodeId))
            }
            // This session's chat only: messages aren't kept after the app stops.
            messages.forEach { m ->
                add(
                    Findable(
                        FoundKind.MESSAGES, m.id.toString(), "", body = m.text, node = m.node, ts = m.ts,
                        tag = if (m.incoming) received else sent,
                    ),
                )
            }
            clips.forEach { c ->
                add(
                    Findable(
                        FoundKind.CLIPBOARD, c.id.toString(), "", body = c.text, node = c.node, ts = c.at,
                        tag = if (c.incoming) received else sent,
                    ),
                )
            }
            transfers.forEach { t ->
                val tag = when (t.state) {
                    TransferState.Active -> underWay
                    TransferState.Failed -> failed
                    TransferState.Done -> if (t.incoming) received else sent
                }
                add(Findable(FoundKind.TRANSFERS, t.id, t.name, node = t.node, ts = t.ended ?: t.started, tag = tag))
            }
            folders.forEach { p ->
                val synced = p.lastSynced.toLong().takeIf { it > 0 }?.let { it * 1000 }
                add(Findable(FoundKind.FOLDERS, p.id, p.name, body = p.path, node = p.peer, ts = synced))
            }
            automations.forEach { a ->
                val sentence = ctx.getString(
                    R.string.automation_arrow, PhoneAutomations.whenText(ctx, a.trigger), PhoneAutomations.thenText(ctx, a),
                )
                val devices = listOf(a.trigger.node, a.node).filter { it.isNotEmpty() }.distinct()
                    .joinToString(" ") { Core.peerName(it) }
                add(
                    Findable(
                        FoundKind.AUTOMATIONS, a.id, a.label.ifBlank { sentence }, devices,
                        if (a.label.isBlank()) "" else sentence,
                        node = a.node.ifEmpty { a.trigger.node }.ifEmpty { null }, ts = a.lastFired,
                    ),
                )
            }
            runs.forEachIndexed { i, r ->
                add(
                    Findable(
                        FoundKind.AUTOMATION_APPS, "${r.ts}-$i", r.what, AutomationAppsGate.appName(ctx, r.caller),
                        AutomationAppsGate.statusText(ctx, r.ok, r.detail), ts = r.ts,
                    ),
                )
            }
            addAll(fixed)
        }
    }
}

/** [text] with the query's words marked (bold on an accent wash). */
@Composable
fun highlighted(text: String, words: List<String>): AnnotatedString {
    val mark = SpanStyle(background = T.accentBg, color = T.accentFg, fontWeight = FontWeight.Bold)
    return buildAnnotatedString {
        append(text)
        for (s in Search.matchRanges(text, words)) addStyle(mark, s.start, s.end)
    }
}

/**
 * Search (from the field at the top of Home): a text field, type chips,
 * and the results grouped by kind (the first [Search.PREVIEW] of each, "Show
 * all N" for the rest). Tapping a result opens it through [onOpen]; the
 * query, chips and open groups live in the shell, so Back comes back here
 * as it was.
 */
@Composable
fun SearchScreen(
    query: String,
    onQuery: (String) -> Unit,
    kinds: Set<FoundKind>,
    onKinds: (Set<FoundKind>) -> Unit,
    expanded: Set<FoundKind>,
    onExpanded: (Set<FoundKind>) -> Unit,
    onOpen: (Findable) -> Unit,
    onBack: () -> Unit,
) {
    val ctx = LocalContext.current
    val focus = LocalFocusManager.current
    val field = remember { FocusRequester() }
    val items = rememberFindables(ctx)
    val groups = remember(query, kinds, items) { Search.search(query, items, kinds) }
    val words = remember(query) { Search.queryWords(query) }
    val total = groups.sumOf { it.total }

    // Straight to typing when it opens empty (not when coming back to results).
    LaunchedEffect(Unit) { if (query.isEmpty()) runCatching { field.requestFocus() } }

    Column(Modifier.fillMaxSize().imePadding()) {
        Row(Modifier.fillMaxWidth().padding(start = 4.dp, top = 4.dp, end = 16.dp), verticalAlignment = Alignment.CenterVertically) {
            IconButton(onClick = onBack) { Icon(Icons.AutoMirrored.Filled.ArrowBack, stringResource(R.string.back), tint = T.text) }
            Text(
                stringResource(R.string.search_title), Modifier.semantics { heading() },
                style = MaterialTheme.typography.headlineSmall, color = T.text,
            )
        }
        OutlinedTextField(
            query, onQuery,
            modifier = Modifier.fillMaxWidth().padding(horizontal = 16.dp).focusRequester(field),
            label = { Text(stringResource(R.string.search_field)) },
            singleLine = true,
            leadingIcon = { Icon(Icons.Filled.Search, contentDescription = null, tint = T.text3) },
            trailingIcon = if (query.isEmpty()) null else {
                {
                    IconButton(onClick = { onQuery(""); runCatching { field.requestFocus() } }) {
                        Icon(Icons.Filled.Clear, stringResource(R.string.search_clear), tint = T.text2)
                    }
                }
            },
            keyboardOptions = KeyboardOptions(imeAction = ImeAction.Search),
            keyboardActions = KeyboardActions(onSearch = { focus.clearFocus() }),
            shape = MaterialTheme.shapes.small,
            colors = OutlinedTextFieldDefaults.colors(
                focusedBorderColor = T.accent, unfocusedBorderColor = T.controlBorder,
                focusedLabelColor = T.accentFg, unfocusedLabelColor = T.text3, cursorColor = T.accent,
                focusedContainerColor = T.surface2, unfocusedContainerColor = T.surface2,
                focusedTextColor = T.text, unfocusedTextColor = T.text,
            ),
        )
        KindChips(kinds, onKinds)

        LazyColumn(
            Modifier.fillMaxSize(),
            contentPadding = PaddingValues(start = 16.dp, end = 16.dp, bottom = 16.dp),
            verticalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            // How many, read out as it changes.
            item(key = "count") {
                val line = when {
                    words.isEmpty() -> stringResource(R.string.search_intro)
                    total == 0 -> stringResource(R.string.search_none, query.trim())
                    else -> pluralStringResource(R.plurals.search_results, total, total)
                }
                Muted(line, Modifier.fillMaxWidth().padding(vertical = 4.dp).announce())
            }
            groups.forEach { g ->
                val open = g.kind in expanded
                item(key = "h-${g.kind}") { GroupHead(g) }
                items(Search.shown(g, open), key = { "r-${g.kind}-${it.item.id}" }) { f ->
                    ResultRow(f, words) { focus.clearFocus(); onOpen(f.item) }
                }
                if (Search.hasMore(g)) {
                    item(key = "m-${g.kind}") {
                        GhostButton(
                            if (open) stringResource(R.string.search_show_fewer)
                            else stringResource(R.string.search_show_all, g.total),
                            { onExpanded(if (open) expanded - g.kind else expanded + g.kind) },
                            small = true,
                        )
                    }
                }
            }
        }
    }
}

/** "All" and one chip per kind; several kinds may be on at once. */
@Composable
private fun KindChips(kinds: Set<FoundKind>, onKinds: (Set<FoundKind>) -> Unit) {
    val colors = FilterChipDefaults.filterChipColors(
        containerColor = T.surface2, labelColor = T.text2,
        selectedContainerColor = T.accentBg, selectedLabelColor = T.accentFg,
    )
    Row(
        Modifier.fillMaxWidth().horizontalScroll(rememberScrollState()).padding(horizontal = 16.dp, vertical = 4.dp),
        horizontalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        // Material keeps a 48 dp touch target around each 32 dp chip.
        FilterChip(
            selected = kinds.isEmpty(), onClick = { onKinds(emptySet()) },
            label = { Text(stringResource(R.string.search_kind_all)) }, colors = colors,
            border = FilterChipDefaults.filterChipBorder(
                enabled = true, selected = kinds.isEmpty(), borderColor = T.borderStrong, selectedBorderColor = T.accent,
            ),
        )
        FoundKind.entries.forEach { k ->
            val on = k in kinds
            FilterChip(
                selected = on, onClick = { onKinds(if (on) kinds - k else kinds + k) },
                label = { Text(stringResource(kindLabel(k))) }, colors = colors,
                border = FilterChipDefaults.filterChipBorder(
                    enabled = true, selected = on, borderColor = T.borderStrong, selectedBorderColor = T.accent,
                ),
            )
        }
    }
}

@Composable
private fun GroupHead(g: FoundGroup) {
    Column(Modifier.fillMaxWidth().padding(top = 12.dp)) {
        Text(
            stringResource(R.string.search_group, stringResource(kindLabel(g.kind)), g.total),
            Modifier.semantics { heading() },
            style = MaterialTheme.typography.titleSmall, color = T.text2,
        )
        // Say so: messages aren't kept, so only this session's are here.
        if (g.kind == FoundKind.MESSAGES) Muted(stringResource(R.string.search_messages_session))
    }
}

/** One result: its title and a line of its text, the words marked, and
 *  where it's from and when. One TalkBack stop; tapping opens it. */
@Composable
private fun ResultRow(f: Found, words: List<String>, onClick: () -> Unit) {
    val item = f.item
    val meta = listOfNotNull(
        item.tag,
        item.node?.takeIf { item.kind != FoundKind.DEVICES }?.let { Core.peerName(it) },
        item.ts?.let { timeAgo(it) },
    ).joinToString(" · ")
    Surface(
        onClick = onClick,
        modifier = Modifier.fillMaxWidth().heightIn(min = 48.dp).semantics { role = Role.Button },
        shape = MaterialTheme.shapes.small,
        color = T.surface,
        border = BorderStroke(1.dp, T.border),
    ) {
        Column(Modifier.padding(horizontal = 14.dp, vertical = 10.dp), verticalArrangement = Arrangement.spacedBy(3.dp)) {
            Text(
                highlighted(f.title, words), style = MaterialTheme.typography.bodyMedium, color = T.text,
                maxLines = 2, overflow = TextOverflow.Ellipsis,
            )
            if (f.detail.isNotEmpty()) {
                Text(
                    highlighted(f.detail, words), style = MaterialTheme.typography.bodySmall, color = T.text2,
                    maxLines = 2, overflow = TextOverflow.Ellipsis,
                )
            }
            if (meta.isNotEmpty()) Muted(meta)
        }
    }
}

/** The Home entry to Search: looks like a field, opens the Search screen. */
@Composable
fun SearchEntry(onOpen: () -> Unit) {
    Surface(
        onClick = onOpen,
        modifier = Modifier.fillMaxWidth().heightIn(min = 48.dp).semantics { role = Role.Button },
        shape = MaterialTheme.shapes.small,
        color = T.surface2,
        border = BorderStroke(1.dp, T.controlBorder),
    ) {
        Row(Modifier.padding(horizontal = 14.dp, vertical = 12.dp), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(10.dp)) {
            Icon(Icons.Filled.Search, contentDescription = null, tint = T.text3)
            Text(stringResource(R.string.search_open), style = MaterialTheme.typography.bodyMedium, color = T.text3)
        }
    }
}

// --- bringing a found item into view --------------------------------------------

/**
 * The row a search result opened: an accent outline, and brought into view
 * once it's laid out. [active] false: nothing.
 */
@OptIn(ExperimentalFoundationApi::class)
@Composable
fun Modifier.foundMark(active: Boolean): Modifier {
    val req = remember { BringIntoViewRequester() }
    LaunchedEffect(active) {
        if (active) {
            // After the screen's first layout (and a lazy list's own scroll).
            delay(300)
            runCatching { req.bringIntoView() }
        }
    }
    return if (active) bringIntoViewRequester(req).border(2.dp, T.accent, MaterialTheme.shapes.medium) else this
}

/** Scroll a lazy list until the item with [key] is on screen (it's only
 *  known once composed), then to it. False when there's no such item. */
suspend fun LazyListState.scrollToKey(key: Any): Boolean {
    snapshotFlow { layoutInfo.totalItemsCount }.first { it > 0 }
    repeat(60) {
        val info = layoutInfo
        info.visibleItemsInfo.firstOrNull { it.key == key }?.let {
            scrollToItem(it.index)
            return true
        }
        val first = info.visibleItemsInfo.firstOrNull()?.index ?: return false
        val last = info.visibleItemsInfo.last().index
        if (last >= info.totalItemsCount - 1) return false
        // On: the last one seen to the top (the next one, if it's all there is).
        scrollToItem(if (last > first) last else last + 1)
    }
    return false
}
