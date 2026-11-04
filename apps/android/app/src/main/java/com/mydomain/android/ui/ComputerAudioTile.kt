package com.mydomain.android.ui

import androidx.compose.ui.Modifier
import androidx.compose.runtime.Composable
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import com.mydomain.android.ComputerAudio
import com.mydomain.android.R

/** Play computer [node]'s ([name]) sound on this phone and the earbuds on
 *  it. Shows its offer when it made one; while it plays, Stop. */
@Composable
fun ComputerAudioTile(node: String, name: String) {
    val ctx = LocalContext.current
    val mine = ComputerAudio.playingFrom == node
    val offered = node in ComputerAudio.offers
    Tile(title = stringResource(R.string.caudio_tile_title)) {
        when {
            mine -> {
                Muted(
                    if (ComputerAudio.flowing) stringResource(R.string.caudio_tile_playing, name)
                    else stringResource(R.string.caudio_tile_waiting, name),
                    Modifier.announce(),
                )
                GhostButton(stringResource(R.string.audio_stop), { ComputerAudio.stop() })
            }
            else -> {
                Muted(
                    if (offered) stringResource(R.string.caudio_tile_offered, name)
                    else stringResource(R.string.caudio_tile_body, name),
                )
                ComputerAudio.lastStop?.let { Muted(it, Modifier.announce()) }
                SoftButton(stringResource(R.string.caudio_tile_start, name), { ComputerAudio.playHere(ctx, node) })
            }
        }
    }
}
