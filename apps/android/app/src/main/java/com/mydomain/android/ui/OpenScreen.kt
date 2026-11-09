package com.mydomain.android.ui

import android.content.ActivityNotFoundException
import android.content.Context
import android.content.Intent
import android.widget.Toast
import androidx.compose.runtime.Composable
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import com.mydomain.android.R

/** Start the first of [intents] this phone can open. False: none could
 *  (no such screen here, or it isn't allowed). */
fun tryOpen(ctx: Context, vararg intents: Intent): Boolean = intents.any { i ->
    try {
        ctx.startActivity(i)
        true
    } catch (_: ActivityNotFoundException) {
        false
    } catch (_: SecurityException) {
        false
    }
}

/** [tryOpen], saying so (a short message) when nothing opened. For code
 *  outside Compose, e.g. an activity. */
fun openOrSay(ctx: Context, vararg intents: Intent): Boolean {
    val ok = tryOpen(ctx, *intents)
    if (!ok) Toast.makeText(ctx, R.string.setup_no_screen, Toast.LENGTH_SHORT).show()
    return ok
}

/** Open one of Android's screens from a button: the first of the intents
 *  that opens, else a message ("This phone has no screen for that"). */
@Composable
fun rememberOpenScreen(): (List<Intent>) -> Unit {
    val ctx = LocalContext.current
    val toast = LocalToast.current
    val none = stringResource(R.string.setup_no_screen)
    return { intents -> if (!tryOpen(ctx, *intents.toTypedArray())) toast(none) }
}
