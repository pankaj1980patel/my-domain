package com.mydomain.android.ui

import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.heading
import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.os.PersistableBundle
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.mydomain.android.AutomationAppsGate
import com.mydomain.android.AutomationAppsKeyReceiver
import com.mydomain.android.R
import com.mydomain.android.core.AutomationApps
import com.mydomain.android.core.Core
import com.mydomain.android.core.timeAgo

/** How many history lines show before "Show all". */
private const val HISTORY_SHOWN = 10

/** Put [text] on the clipboard, marked sensitive: Android hides its
 *  preview, and NetService doesn't send it to your computers. */
private fun copySensitive(ctx: Context, text: String) {
    val clip = ClipData.newPlainText("my-domain", text)
    clip.description.extras = PersistableBundle().apply { putBoolean("android.content.extra.IS_SENSITIVE", true) }
    ctx.getSystemService(ClipboardManager::class.java)?.setPrimaryClip(clip)
}

/** One copyable example of a Tasker "Send Intent" for the key receiver. */
private data class IntentExample(val title: Int, val action: String, val extras: List<String>)

private fun exampleText(ctx: Context, ex: IntentExample, key: String): String = buildString {
    appendLine("Action: ${ex.action}")
    appendLine("Package: ${ctx.packageName}")
    appendLine("Class: ${AutomationAppsKeyReceiver::class.java.name}")
    appendLine("Target: Broadcast Receiver")
    appendLine("Data: ${AutomationApps.KEY_SCHEME}:$key")
    ex.extras.forEachIndexed { i, e -> if (i < ex.extras.size - 1) appendLine("Extra: $e") else append("Extra: $e") }
}

/**
 * Settings → Automation apps: the switch (off by default), the key for
 * Tasker and the like, the apps granted the permission, how to use it with
 * copyable examples, and the history of what apps asked for
 * (AutomationAppsReceiver.kt).
 */
@Composable
fun AutomationAppsTile() {
    val ctx = LocalContext.current
    val toast = LocalToast.current
    LaunchedEffect(Unit) { AutomationAppsGate.load(ctx) }
    var on by remember { mutableStateOf(AutomationAppsGate.enabled(ctx)) }
    var key by remember { mutableStateOf(if (on) AutomationAppsGate.key(ctx) else null) }
    var showKey by remember { mutableStateOf(false) }
    var howTo by remember { mutableStateOf(false) }
    var allHistory by remember { mutableStateOf(false) }
    var confirmNewKey by remember { mutableStateOf(false) }
    val copied = stringResource(R.string.automation_apps_copied)

    Tile(title = stringResource(R.string.automation_apps_title)) {
        Muted(stringResource(R.string.automation_apps_hint))
        SwitchRow(stringResource(R.string.automation_apps_switch), on) {
            AutomationAppsGate.setEnabled(ctx, it)
            on = it
            key = if (it) AutomationAppsGate.key(ctx) else null
        }
        if (!on) return@Tile

        // The key, for apps that can't hold the permission.
        Text(stringResource(R.string.automation_apps_key), Modifier.semantics { heading() }, style = MaterialTheme.typography.titleSmall, color = T.text)
        Muted(stringResource(R.string.automation_apps_key_hint))
        val k = key.orEmpty()
        val hidden = stringResource(R.string.a11y_key_hidden)
        Text(
            if (showKey) k else "•".repeat(16),
            // Hidden, TalkBack says so rather than "bullet" sixteen times.
            if (showKey) Modifier else Modifier.clearAndSetSemantics { contentDescription = hidden },
            fontFamily = FontFamily.Monospace, fontSize = 13.sp, color = T.text,
        )
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            GhostButton(
                stringResource(if (showKey) R.string.automation_apps_hide_key else R.string.automation_apps_show_key),
                { showKey = !showKey }, small = true,
            )
            GhostButton(stringResource(R.string.automation_apps_copy), {
                copySensitive(ctx, k)
                toast(copied)
            }, small = true)
            GhostButton(stringResource(R.string.automation_apps_new_key), { confirmNewKey = true }, small = true)
        }
        if (confirmNewKey) {
            Muted(stringResource(R.string.automation_apps_new_key_confirm))
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                PrimaryButton(stringResource(R.string.automation_apps_new_key), {
                    key = AutomationAppsGate.newKey(ctx)
                    confirmNewKey = false
                })
                GhostButton(stringResource(R.string.cancel), { confirmNewKey = false })
            }
        }

        // Apps Android has given the permission to (no key needed).
        Text(stringResource(R.string.automation_apps_allowed), Modifier.semantics { heading() }, style = MaterialTheme.typography.titleSmall, color = T.text)
        val allowed = remember(on) { AutomationAppsGate.allowedApps(ctx) }
        Muted(
            if (allowed.isEmpty()) stringResource(R.string.automation_apps_allowed_none)
            else allowed.joinToString(", ") { AutomationAppsGate.appName(ctx, it) },
        )
        Muted(stringResource(R.string.automation_apps_allowed_hint))

        // How to use it, with examples to copy.
        GhostButton(
            stringResource(if (howTo) R.string.automation_apps_how_hide else R.string.automation_apps_how),
            { howTo = !howTo }, small = true,
        )
        if (howTo) {
            Muted(stringResource(R.string.automation_apps_how_steps))
            val device = Core.peers.firstOrNull { it.nodeId != Core.me?.nodeId }?.let { Core.peerName(it.nodeId) } ?: stringResource(R.string.automation_apps_example_device)
            val examples = listOf(
                IntentExample(R.string.automation_apps_example_lock, AutomationApps.ACTION_RUN, listOf("device:$device", "action:lock")),
                IntentExample(
                    R.string.automation_apps_example_mute, AutomationApps.ACTION_RUN,
                    listOf("device:$device", "action:mute", "args:{\"on\":true}"),
                ),
                IntentExample(R.string.automation_apps_example_text, AutomationApps.ACTION_SEND_TEXT, listOf("device:$device", "text:%CLIP", "as:clipboard")),
                IntentExample(R.string.automation_apps_example_automation, AutomationApps.ACTION_RUN_AUTOMATION, listOf("automation:Silence")),
            )
            examples.forEach { ex ->
                val text = exampleText(ctx, ex, k)
                Text(stringResource(ex.title), style = MaterialTheme.typography.bodyMedium, color = T.text)
                Text(text, style = MaterialTheme.typography.bodySmall.copy(fontFamily = FontFamily.Monospace), color = T.text2)
                GhostButton(stringResource(R.string.automation_apps_copy), {
                    copySensitive(ctx, text)
                    toast(copied)
                }, small = true)
            }
            Muted(stringResource(R.string.automation_apps_how_more))
        }

        // What apps asked for.
        Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
            Text(
                stringResource(R.string.automation_apps_history), style = MaterialTheme.typography.titleSmall, color = T.text,
                modifier = Modifier.weight(1f).semantics { heading() },
            )
            if (AutomationAppsGate.history.isNotEmpty()) {
                GhostButton(stringResource(R.string.automation_apps_clear), { AutomationAppsGate.clearHistory(ctx) }, small = true)
            }
        }
        val history = AutomationAppsGate.history
        if (history.isEmpty()) Muted(stringResource(R.string.automation_apps_history_empty))
        val shown = if (allHistory) history.toList() else history.take(HISTORY_SHOWN)
        shown.forEach { r ->
            Column(Modifier.semantics(mergeDescendants = true) {}, verticalArrangement = Arrangement.spacedBy(2.dp)) {
                Text(r.what, style = MaterialTheme.typography.bodyMedium, color = T.text)
                Muted(
                    "${AutomationAppsGate.appName(ctx, r.caller)} · ${timeAgo(r.ts)} · " +
                        AutomationAppsGate.statusText(ctx, r.ok, r.detail),
                )
            }
        }
        if (!allHistory && history.size > HISTORY_SHOWN) {
            SoftButton(stringResource(R.string.automation_apps_show_all, history.size), { allHistory = true })
        }
    }
}
