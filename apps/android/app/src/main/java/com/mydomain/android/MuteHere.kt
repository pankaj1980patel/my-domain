package com.mydomain.android

import android.content.Context
import android.media.AudioManager
import android.util.Log
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import com.mydomain.android.core.MuteHerePlan

/**
 * "Mute this phone while it plays on <computer>": a switch per computer,
 * remembered. While this phone's audio plays on a computer with the switch
 * on, the phone's own media sound (the music stream) is muted, so it comes
 * out of the computer only.
 *
 * Android's playback capture (PhoneAudio.kt) takes the apps' audio before
 * the stream volume and mute, so muting here doesn't silence what the
 * computer hears.
 *
 * Restored when the last such computer stops (Stop, the link lost, the
 * capture ended); what was changed is written down first, so after a crash
 * NetService's start undoes it ([restore]). A stream that was muted before,
 * or unmuted by hand meanwhile, is left alone ([MuteHerePlan]).
 */
object MuteHere {
    private const val TAG = "mydomain_mute"
    private const val PREFS = "mute_here"
    private const val KEY_ACTIVE = "active"

    /** This app has the phone's sound muted now (the tile says so). */
    var active by mutableStateOf(false)
        private set

    private fun prefs(ctx: Context) = ctx.getSharedPreferences(PREFS, Context.MODE_PRIVATE)

    fun switchOn(ctx: Context, node: String): Boolean =
        prefs(ctx).getBoolean("on:$node", false)

    fun setSwitch(ctx: Context, node: String, on: Boolean) {
        prefs(ctx).edit().putBoolean("on:$node", on).apply()
        sync(ctx, PhoneAudio.playingTo)
    }

    private fun audio(ctx: Context) = ctx.getSystemService(AudioManager::class.java)

    private fun muted(am: AudioManager): Boolean = am.isStreamMute(AudioManager.STREAM_MUSIC)

    private fun set(am: AudioManager, on: Boolean) {
        val dir = if (on) AudioManager.ADJUST_MUTE else AudioManager.ADJUST_UNMUTE
        am.adjustStreamVolume(AudioManager.STREAM_MUSIC, dir, 0)
    }

    /** Bring the phone's sound in line with where its audio plays now. */
    @Synchronized
    fun sync(ctx: Context, playingTo: Set<String>) {
        val app = ctx.applicationContext
        val want = MuteHerePlan.wanted(playingTo) { switchOn(app, it) }
        val saved = prefs(app).getString(KEY_ACTIVE, null)
        if (want && saved == null) {
            val am = audio(app) ?: return
            val (setTo, keep) = MuteHerePlan.begin(muted(am))
            // Written down first: a crash after muting still finds it.
            prefs(app).edit().putString(KEY_ACTIVE, keep).commit()
            if (setTo != null) {
                runCatching { set(am, setTo) }.onFailure {
                    Log.w(TAG, "can't mute: $it")
                    prefs(app).edit().remove(KEY_ACTIVE).commit()
                    return
                }
            }
            active = keep == MuteHerePlan.MUTED_BY_US
        } else if (!want && saved != null) {
            restore(app)
        }
    }

    /** Undo what this app muted (on stop, and at start after a crash). */
    @Synchronized
    fun restore(ctx: Context) {
        val app = ctx.applicationContext
        val saved = prefs(app).getString(KEY_ACTIVE, null) ?: return
        val am = audio(app)
        if (am != null) {
            MuteHerePlan.end(saved, muted(am))?.let { on ->
                runCatching { set(am, on) }.onFailure { Log.w(TAG, "can't unmute: $it") }
            }
        }
        prefs(app).edit().remove(KEY_ACTIVE).commit()
        active = false
    }
}
