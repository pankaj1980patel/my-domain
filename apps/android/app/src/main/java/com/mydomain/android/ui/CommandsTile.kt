package com.mydomain.android.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.unit.dp
import com.mydomain.android.R
import com.mydomain.android.core.Core
import com.mydomain.android.core.LastRun
import com.mydomain.android.core.friendlyError
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import org.json.JSONObject

/** How often to ask for the result while one runs. */
private const val POLL_MS = 1500L

/** Give up waiting after this (the computer stops a command after 60 s). */
private const val WAIT_MS = 75_000L

/**
 * A computer's commands: the ones its owner wrote in its Settings, listed
 * in its status (`commands`). Run sends only the command's id (`run {id}`);
 * the computer runs it and puts how it went in its status (`last_run`),
 * which this tile asks for until it changes.
 */
@Composable
fun CommandsTile(node: String, name: String) {
    val toast = LocalToast.current
    val ctx = LocalContext.current
    val scope = rememberCoroutineScope()
    var running by remember { mutableStateOf<String?>(null) }
    val st = Core.deviceStatus[node]
    val commands = st?.commands.orEmpty()

    fun run(id: String, what: String) {
        // The result is new once `last_run` changes from what it is now (its
        // `at` is the computer's clock, which may differ from this one's).
        val before = Core.deviceStatus[node]?.lastRun
        running = id
        scope.launch {
            try {
                val via = Core.runAction(node, "run", JSONObject().put("id", id))
                toast(
                    if (via == "queued") ctx.getString(R.string.run_queued, name)
                    else ctx.getString(R.string.run_asked, name, what)
                )
                val started = System.currentTimeMillis()
                while (true) {
                    val last = Core.deviceStatus[node]?.lastRun
                    if (last != null && last.id == id && last != before) {
                        last.error?.let { toast(ctx.getString(R.string.run_refused, name, it)) }
                        break
                    }
                    if (System.currentTimeMillis() - started > WAIT_MS) {
                        toast(ctx.getString(R.string.run_no_answer, name))
                        break
                    }
                    delay(POLL_MS)
                    runCatching { Core.requestStatus(node) }
                }
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                toast(friendlyError(e))
            } finally {
                running = null
            }
        }
    }

    Tile(title = stringResource(R.string.run_title)) {
        Muted(stringResource(R.string.run_hint, name))
        commands.forEach { c ->
            Row(verticalAlignment = Alignment.CenterVertically) {
                Text(c.name, style = MaterialTheme.typography.bodyMedium, color = T.text, modifier = Modifier.weight(1f))
                GhostButton(
                    stringResource(if (running == c.id) R.string.run_running else R.string.run_run),
                    { if (running == null) run(c.id, c.name) },
                    small = true,
                )
            }
        }
        st?.lastRun?.takeIf { it.error == null }?.let { RunResult(it) }
    }
}

/** How a run went: the exit code, then the end of what it printed. */
@Composable
private fun RunResult(r: LastRun) {
    val line = when {
        r.timedOut -> stringResource(R.string.run_timed_out, r.name)
        r.code != null -> stringResource(R.string.run_exit_code, r.name, r.code)
        else -> stringResource(R.string.run_stopped, r.name)
    }
    Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
        Text(
            line, Modifier.announce(), style = MaterialTheme.typography.labelMedium,
            color = if (r.code == 0) T.text2 else T.danger,
        )
        Text(
            r.output.ifEmpty { stringResource(R.string.run_no_output) },
            style = MaterialTheme.typography.bodySmall,
            fontFamily = FontFamily.Monospace,
            color = T.text,
            modifier = Modifier
                .fillMaxWidth()
                .heightIn(max = 200.dp)
                .background(T.surface2, MaterialTheme.shapes.small)
                .verticalScroll(rememberScrollState())
                .padding(8.dp),
        )
    }
}
