package com.mydomain.android.ui

import android.content.Intent
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.mydomain.android.CrashLog
import com.mydomain.android.R

/**
 * "Crash log" (Settings, in every build; the Developer screen too): shown
 * only once something was kept. Opens the log, to share (it never leaves
 * the phone otherwise) or clear.
 */
@Composable
fun CrashLogRow() {
    val ctx = LocalContext.current
    var text by remember { mutableStateOf(CrashLog.read(ctx)) }
    var open by remember { mutableStateOf(false) }
    if (text.isEmpty()) return
    RowGroup {
        NavRow(
            R.drawable.ic_lucide_triangle_alert, Hue.Amber,
            stringResource(R.string.crash_log_title), stringResource(R.string.crash_log_hint),
            onClick = { text = CrashLog.read(ctx); open = true },
        )
    }
    if (open) CrashLogDialog(text, close = { open = false }, cleared = { text = ""; open = false })
}

@Composable
private fun CrashLogDialog(text: String, close: () -> Unit, cleared: () -> Unit) {
    val ctx = LocalContext.current
    val toast = LocalToast.current
    val shareTitle = stringResource(R.string.crash_log_title)
    val noApp = stringResource(R.string.crash_log_share_failed)
    AlertDialog(
        onDismissRequest = close,
        containerColor = T.bar,
        title = { Text(stringResource(R.string.crash_log_title), color = T.text) },
        text = {
            SelectionContainer {
                Text(
                    text,
                    Modifier.heightIn(max = 420.dp).verticalScroll(rememberScrollState()),
                    fontFamily = FontFamily.Monospace, fontSize = 11.sp, color = T.text2,
                )
            }
        },
        confirmButton = {
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                GhostButton(stringResource(R.string.crash_log_clear), {
                    CrashLog.clear(ctx)
                    cleared()
                })
                PrimaryButton(stringResource(R.string.crash_log_share), {
                    val send = Intent(Intent.ACTION_SEND).setType("text/plain")
                        .putExtra(Intent.EXTRA_SUBJECT, "my-domain $shareTitle")
                        .putExtra(Intent.EXTRA_TEXT, text)
                    runCatching { ctx.startActivity(Intent.createChooser(send, shareTitle)) }
                        .onFailure { toast(noApp) }
                })
            }
        },
        dismissButton = { GhostButton(stringResource(R.string.crash_log_close), close) },
    )
}
