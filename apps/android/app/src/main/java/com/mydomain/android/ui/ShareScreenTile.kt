package com.mydomain.android.ui

import androidx.compose.ui.Modifier
import androidx.compose.runtime.Composable
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import com.mydomain.android.PhoneHelper
import com.mydomain.android.R
import com.mydomain.android.ScreenShare

/** Share this phone's screen with computer [node] ([name]): Android's own
 *  prompt first, every time; then the computer shows it. While it shares:
 *  Stop, and whether the computer may tap and swipe here (off at each
 *  start; needs the Accessibility helper). */
@Composable
fun ShareScreenTile(node: String, name: String) {
    val ctx = LocalContext.current
    val sharing = node in ScreenShare.sharingWith
    Tile(title = stringResource(R.string.screen_share_title)) {
        if (!sharing) {
            Muted(stringResource(R.string.screen_share_body, name))
            SoftButton(stringResource(R.string.screen_share_start, name), { ScreenShare.share(ctx, node) })
        } else {
            Muted(stringResource(R.string.screen_share_now, name), Modifier.announce())
            NearbyMediaBadge(node)
            if (PhoneHelper.instance != null) {
                SwitchRow(stringResource(R.string.screen_share_allow, name), ScreenShare.control) { ScreenShare.allowControl(it) }
                Muted(stringResource(R.string.screen_share_allow_hint, name))
            } else {
                Muted(stringResource(R.string.screen_share_needs_helper, name))
            }
            GhostButton(stringResource(R.string.screen_share_stop), { ScreenShare.stopWith(node) })
        }
    }
}
