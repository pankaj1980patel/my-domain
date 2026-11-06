package com.mydomain.android.core

import android.content.ClipData
import android.content.ClipDescription
import android.content.Context
import android.os.PersistableBundle

/**
 * Clipboard privacy. A copy marked sensitive (a password manager’s, which
 * sets `ClipDescription.EXTRA_IS_SENSITIVE`, or the automation-apps key
 * from Settings) stays on this phone: not synced, not kept in the clipboard
 * history. A clip a computer sent marked sensitive (`text_shared` with
 * `sensitive: true`) goes on the clipboard marked so, which keyboards and
 * Android’s own clipboard preview hide, and is kept out of the history.
 *
 * "Never sync passwords and other concealed clips" in Settings: on by
 * default; pref [PREF] in "md".
 */
object ClipPrivacy {
    const val PREF = "clip_hide_concealed"

    /** `ClipDescription.EXTRA_IS_SENSITIVE` (API 33); the same key is read
     *  by password managers and keyboards on older versions too. */
    const val EXTRA_IS_SENSITIVE = "android.content.extra.IS_SENSITIVE"

    fun hideConcealed(ctx: Context): Boolean =
        ctx.getSharedPreferences("md", Context.MODE_PRIVATE).getBoolean(PREF, true)

    fun setHideConcealed(ctx: Context, on: Boolean) {
        ctx.getSharedPreferences("md", Context.MODE_PRIVATE).edit().putBoolean(PREF, on).apply()
    }

    /** Whether a copy made here stays here: marked [sensitive] while the
     *  setting is on ([hide]). */
    fun keepHere(sensitive: Boolean, hide: Boolean): Boolean = sensitive && hide

    /** Whether a clip with this [description] is marked sensitive. */
    fun isSensitive(description: ClipDescription?): Boolean =
        description?.extras?.getBoolean(EXTRA_IS_SENSITIVE) == true

    /** Plain text for the clipboard, marked sensitive when [sensitive]. */
    fun clipOf(label: String, text: String, sensitive: Boolean): ClipData =
        ClipData.newPlainText(label, text).apply {
            if (sensitive) description.extras = PersistableBundle().apply { putBoolean(EXTRA_IS_SENSITIVE, true) }
        }
}
