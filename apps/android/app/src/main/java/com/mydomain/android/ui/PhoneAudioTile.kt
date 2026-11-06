package com.mydomain.android.ui

import androidx.compose.ui.Modifier
import androidx.compose.runtime.Composable
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import com.mydomain.android.PhoneAudio
import com.mydomain.android.R

/** Play this phone's media audio on computer [node] ([name]): the
 *  microphone permission (once) and Android's own prompt (every time)
 *  first. While it plays: what's heard and what isn't, and Stop. */
@Composable
fun PhoneAudioTile(node: String, name: String) {
    val ctx = LocalContext.current
    val playing = node in PhoneAudio.playingTo
    Tile(title = stringResource(R.string.audio_tile_title)) {
        if (!playing) {
            Muted(stringResource(R.string.audio_tile_body, name))
            SoftButton(stringResource(R.string.audio_tile_start, name), { PhoneAudio.playOn(ctx, node) })
        } else {
            Muted(stringResource(R.string.audio_tile_now, name), Modifier.announce())
            NearbyMediaBadge(node)
            Muted(stringResource(R.string.audio_tile_hint))
            GhostButton(stringResource(R.string.audio_stop), { PhoneAudio.stopWith(node) })
        }
    }
}
