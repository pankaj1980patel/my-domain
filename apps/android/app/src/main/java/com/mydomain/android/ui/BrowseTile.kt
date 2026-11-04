package com.mydomain.android.ui

import androidx.compose.foundation.layout.heightIn
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.semantics
import android.text.format.DateUtils
import android.text.format.Formatter
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateListOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import com.mydomain.android.R
import com.mydomain.android.core.Core
import com.mydomain.android.core.RemoteFile
import com.mydomain.android.core.friendlyError
import kotlinx.coroutines.launch

/**
 * Another device's shared folders, from the phone (the desktop's "Browse
 * files" the other way round): its shared folders, then into folders with
 * a path and Up, Load more for long ones, and Download per file (it lands
 * in Downloads/my-domain and shows under Files like any received file).
 * Closed until you open it, so the device page doesn't ask on every visit.
 */
@Composable
fun BrowseTile(node: String, name: String) {
    val toast = LocalToast.current
    val scope = rememberCoroutineScope()
    val ctx = LocalContext.current
    var open by remember { mutableStateOf(false) }
    // Where we are: "" root = the list of shared folders.
    var root by remember { mutableStateOf("") }
    var path by remember { mutableStateOf("") }
    var req by remember { mutableStateOf<String?>(null) }
    val entries = remember { mutableStateListOf<RemoteFile>() }
    var next by remember { mutableStateOf<Long?>(null) }
    var error by remember { mutableStateOf<String?>(null) }
    var waiting by remember { mutableStateOf(false) }
    val downloading = stringResource(R.string.browse_downloading)

    fun list(r: String, p: String, offset: Long = 0) {
        if (offset == 0L) {
            entries.clear()
            next = null
        }
        root = r
        path = p
        error = null
        waiting = true
        scope.launch {
            try {
                req = Core.listFolder(node, r, p, offset)
            } catch (e: Exception) {
                waiting = false
                error = friendlyError(e)
            }
        }
    }

    // The answer arrives in Core.listings under our request id.
    val answer = req?.let { Core.listings[it] }
    LaunchedEffect(answer) {
        answer ?: return@LaunchedEffect
        waiting = false
        error = answer.error
        entries.addAll(answer.entries.sortedWith(compareBy({ !it.dir }, { it.name.lowercase() })))
        next = answer.next
    }

    Tile(title = stringResource(R.string.browse_title)) {
        if (!open) {
            SoftButton(stringResource(R.string.browse_open, name), {
                open = true
                list("", "")
            }, Modifier.fillMaxWidth())
            return@Tile
        }
        // Where we are, and the way back up.
        Row(verticalAlignment = Alignment.CenterVertically) {
            Text(
                if (root.isEmpty()) stringResource(R.string.browse_shared, name) else listOf(root, path).filter { it.isNotEmpty() }.joinToString(" / "),
                style = MaterialTheme.typography.bodyMedium, color = T.text, modifier = Modifier.weight(1f).announce(),
            )
            if (root.isNotEmpty()) {
                GhostButton(stringResource(R.string.browse_up), {
                    if (path.isEmpty()) list("", "") else list(root, path.substringBeforeLast('/', ""))
                }, small = true)
            }
            GhostButton(stringResource(R.string.browse_close), { open = false }, small = true)
        }
        error?.let { Muted(browseError(it), Modifier.announce()) }
        if (waiting && entries.isEmpty()) Muted(stringResource(R.string.browse_loading), Modifier.announce())
        if (!waiting && error == null && entries.isEmpty()) {
            Muted(stringResource(if (root.isEmpty()) R.string.browse_none_shared else R.string.browse_empty))
        }
        Column(verticalArrangement = Arrangement.spacedBy(2.dp)) {
            entries.forEach { e ->
                Row(
                    // A folder row opens it: a button, 48 dp tall at least.
                    Modifier.fillMaxWidth().heightIn(min = 48.dp)
                        .semantics(mergeDescendants = true) {}
                        .clickable(enabled = e.dir || root.isEmpty(), role = Role.Button) {
                            when {
                                root.isEmpty() -> list(e.name, "")
                                e.dir -> list(root, if (path.isEmpty()) e.name else "$path/${e.name}")
                            }
                        }
                        .padding(vertical = 6.dp),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    Column(Modifier.weight(1f)) {
                        Text(if (e.dir || root.isEmpty()) "${e.name}/" else e.name, style = MaterialTheme.typography.bodyMedium, color = T.text)
                        if (!e.dir && root.isNotEmpty()) {
                            val date = if (e.modified > 0) DateUtils.getRelativeTimeSpanString(e.modified * 1000).toString() else ""
                            Muted(listOf(Formatter.formatShortFileSize(ctx, e.size), date).filter { it.isNotEmpty() }.joinToString(" · "))
                        }
                    }
                    if (!e.dir && root.isNotEmpty()) {
                        GhostButton(stringResource(R.string.browse_download), {
                            val full = if (path.isEmpty()) e.name else "$path/${e.name}"
                            scope.launch {
                                try {
                                    Core.downloadFile(node, root, full)
                                    toast(downloading)
                                } catch (ex: Exception) { toast(friendlyError(ex)) }
                            }
                        }, small = true)
                    }
                }
            }
        }
        next?.let { n ->
            GhostButton(stringResource(R.string.browse_more), { list(root, path, n) })
        }
    }
}

/** A device's reason, in plain words where we know it. */
@Composable
private fun browseError(raw: String): String {
    val r = raw.lowercase()
    return when {
        "not shared" in r -> stringResource(R.string.browse_err_not_shared)
        "not found" in r || "missing" in r -> stringResource(R.string.browse_err_missing)
        "permission" in r || "can't read" in r -> stringResource(R.string.browse_err_access)
        else -> raw
    }
}
