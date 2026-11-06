package com.mydomain.android

import android.Manifest
import android.annotation.SuppressLint
import android.app.Activity
import android.app.AlertDialog
import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.content.pm.ServiceInfo
import android.media.AudioAttributes
import android.media.AudioFormat
import android.media.AudioPlaybackCaptureConfiguration
import android.media.AudioRecord
import android.media.projection.MediaProjection
import android.media.projection.MediaProjectionManager
import android.os.Build
import android.os.Bundle
import android.os.Handler
import android.os.IBinder
import android.os.Looper
import android.util.Log
import android.widget.Toast
import androidx.annotation.RequiresApi
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.core.app.NotificationCompat
import com.mydomain.android.core.AudioPlan
import com.mydomain.android.core.Core
import com.mydomain.android.core.ResourceBook
import com.mydomain.android.core.ResourceLedger

/**
 * This phone's media audio (music, videos, podcasts; never calls) played on
 * a computer's speakers (mdcore `features::audio_share`). Two ways in:
 *
 *  - the computer asks ("Play this phone's audio here"): the phone asks you
 *    (a prompt on screen, else a notification);
 *  - you tap "Play on …" on a computer's page here.
 *
 * Either way: the microphone permission (asked here with why, once; playback
 * capture needs it, nothing is recorded from the microphone), then
 * Android's own "start recording or casting?" prompt, every time. Nothing
 * is captured without both. Android 10 or later only (playback capture);
 * older phones don't offer it.
 *
 * What's captured: what apps play as media, games and unknown usage. Apps
 * that forbid capture (and apps targeting Android 9 or older that didn't
 * allow it) stay silent. The phone keeps playing it too: turn its volume
 * down if you like (Android gives no clean way to mute only what's
 * captured). It keeps going with the screen off, and stops on Stop (here,
 * the notification or the computer), when the computer stops listening or
 * its link goes, or when Android ends the capture.
 *
 * One capture at a time: while the screen is shared it refuses (a second
 * MediaProjection would end the first), and the screen share refuses while
 * this plays.
 */
object PhoneAudio {
    private const val TAG = "mydomain_audio"
    const val CH = "phone_audio"
    const val NOTE_ASK = 7311
    const val NOTE_PLAYING = 7312

    val supported: Boolean get() = Build.VERSION.SDK_INT >= AudioPlan.MIN_SDK

    /** Computers this phone's audio plays on now (empty: not playing). */
    var playingTo by mutableStateOf<Set<String>>(emptySet())
        private set

    private val main = Handler(Looper.getMainLooper())

    internal fun published(targets: Set<String>) {
        main.post { playingTo = targets }
        // Mute here follows where it plays (MuteHere.kt).
        runCatching { MuteHere.sync(Core.app, targets) }
    }

    /** A computer started ([on]) or stopped listening to this phone. */
    fun onListen(ctx: Context, from: String, on: Boolean) {
        val svc = PhoneAudioService.current
        if (!on) {
            svc?.targetLeft(from)
            return
        }
        if (svc != null && svc.isTarget(from)) return
        val app = ctx.applicationContext
        channel(app)
        val ask = Intent(app, PhoneAudioConsentActivity::class.java)
            .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
            .putExtra(PhoneAudioConsentActivity.EXTRA_NODE, from)
        if (svc == null && Core.visible && runCatching { app.startActivity(ask) }.isSuccess) return
        val pi = PendingIntent.getActivity(app, NOTE_ASK, ask, PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
        val n = NotificationCompat.Builder(app, CH)
            .setSmallIcon(android.R.drawable.ic_lock_silent_mode_off)
            .setContentTitle(app.getString(R.string.audio_ask_title, Core.peerName(from)))
            .setContentText(app.getString(R.string.audio_ask_body))
            .setPriority(NotificationCompat.PRIORITY_HIGH)
            .setAutoCancel(true)
            .setContentIntent(pi)
            .build()
        runCatching { app.getSystemService(NotificationManager::class.java)?.notify(NOTE_ASK, n) }
            .onFailure { Log.w(TAG, "can't ask: $it") }
    }

    /** You chose to play this phone's audio on computer [node] (from its
     *  page; [ctx] is the activity). */
    fun playOn(ctx: Context, node: String) {
        val svc = PhoneAudioService.current
        if (svc != null) {
            svc.addTarget(node, offer = true)
            return
        }
        ctx.startActivity(
            Intent(ctx, PhoneAudioConsentActivity::class.java)
                .putExtra(PhoneAudioConsentActivity.EXTRA_NODE, node)
                .putExtra(PhoneAudioConsentActivity.EXTRA_OFFER, true),
        )
    }

    /** Stop playing on [node] (the last one: stop). */
    fun stopWith(node: String) {
        PhoneAudioService.current?.targetLeft(node, tell = true)
    }

    /** The link to [node] went: it stopped playing there. */
    fun linkLost(node: String) {
        PhoneAudioService.current?.targetLeft(node)
    }

    /** Tell computer [node] it won't get this phone's audio (you said no,
     *  or it can't start), so it stops waiting. */
    fun decline(node: String) {
        if (node.isEmpty() || !(Core.engine()?.audioSink(node) ?: false)) return
        Thread { runCatching { Core.engine()?.audioOffer(node, false) } }.start()
    }

    fun channel(ctx: Context) {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.O) return
        ctx.getSystemService(NotificationManager::class.java)?.createNotificationChannel(
            NotificationChannel(CH, ctx.getString(R.string.channel_audio), NotificationManager.IMPORTANCE_HIGH),
        )
    }

    /** "Playing this phone's audio on …", with Stop. */
    fun playingNote(ctx: Context, targets: Set<String>): Notification {
        val stop = PendingIntent.getService(
            ctx, 0, Intent(ctx, PhoneAudioService::class.java).setAction(PhoneAudioService.ACTION_STOP),
            PendingIntent.FLAG_IMMUTABLE,
        )
        val names = targets.joinToString(", ") { Core.peerName(it) }
        return NotificationCompat.Builder(ctx, CH)
            .setSmallIcon(android.R.drawable.ic_lock_silent_mode_off)
            .setContentTitle(
                if (names.isEmpty()) ctx.getString(R.string.audio_playing_title)
                else ctx.getString(R.string.audio_playing_on, names),
            )
            .setContentText(ctx.getString(R.string.audio_playing_body))
            .setStyle(NotificationCompat.BigTextStyle().bigText(ctx.getString(R.string.audio_playing_body)))
            .setOngoing(true)
            .setOnlyAlertOnce(true)
            .addAction(0, ctx.getString(R.string.audio_stop), stop)
            .build()
    }
}

/** Asks for the microphone permission (with why), then shows Android's
 *  recording prompt; a yes starts the audio. */
class PhoneAudioConsentActivity : Activity() {
    private var node = ""
    private var offer = false

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        getSystemService(NotificationManager::class.java)?.cancel(PhoneAudio.NOTE_ASK)
        node = intent?.getStringExtra(EXTRA_NODE).orEmpty()
        offer = intent?.getBooleanExtra(EXTRA_OFFER, false) == true
        if (node.isEmpty()) return finish()
        // Already playing (Android said yes then): you tapped to play on
        // this computer too.
        PhoneAudioService.current?.let { svc ->
            svc.addTarget(node, offer = true)
            return finish()
        }
        when (AudioPlan.nextStep(Build.VERSION.SDK_INT, ScreenShareService.running, hasMic())) {
            AudioPlan.Step.Unsupported -> refuse(R.string.audio_unsupported)
            AudioPlan.Step.ScreenBusy -> refuse(R.string.audio_busy_screen)
            AudioPlan.Step.AskPermission -> explainThenAsk()
            AudioPlan.Step.AskConsent -> askAndroid()
        }
    }

    private fun hasMic() =
        checkSelfPermission(Manifest.permission.RECORD_AUDIO) == PackageManager.PERMISSION_GRANTED

    private fun refuse(why: Int) {
        Toast.makeText(this, getString(why), Toast.LENGTH_LONG).show()
        PhoneAudio.decline(node)
        finish()
    }

    /** Why the microphone permission, before Android asks for it. */
    private fun explainThenAsk() {
        AlertDialog.Builder(this, android.R.style.Theme_DeviceDefault_Dialog_Alert)
            .setTitle(R.string.audio_mic_title)
            .setMessage(getString(R.string.audio_mic_body, Core.peerName(node)))
            .setPositiveButton(R.string.audio_mic_continue) { _, _ ->
                requestPermissions(arrayOf(Manifest.permission.RECORD_AUDIO), ASK_MIC)
            }
            .setNegativeButton(R.string.audio_mic_not_now) { _, _ ->
                PhoneAudio.decline(node)
                finish()
            }
            .setOnCancelListener {
                PhoneAudio.decline(node)
                finish()
            }
            .show()
    }

    override fun onRequestPermissionsResult(requestCode: Int, permissions: Array<out String>, grantResults: IntArray) {
        super.onRequestPermissionsResult(requestCode, permissions, grantResults)
        if (requestCode != ASK_MIC) return
        if (grantResults.firstOrNull() == PackageManager.PERMISSION_GRANTED) {
            askAndroid()
        } else {
            refuse(R.string.audio_mic_denied)
        }
    }

    private fun askAndroid() {
        val mpm = getSystemService(MediaProjectionManager::class.java) ?: return refuse(R.string.audio_unsupported)
        @Suppress("DEPRECATION")
        startActivityForResult(mpm.createScreenCaptureIntent(), ASK_CAPTURE)
    }

    @Deprecated("Activity result API; this activity has no other result")
    override fun onActivityResult(requestCode: Int, resultCode: Int, data: Intent?) {
        @Suppress("DEPRECATION")
        super.onActivityResult(requestCode, resultCode, data)
        if (requestCode != ASK_CAPTURE) return
        Log.i("mydomain_audio", "prompt answered: ${if (resultCode == RESULT_OK) "yes" else "no ($resultCode)"}")
        if (resultCode == RESULT_OK && data != null && !ScreenShareService.running) {
            val start = Intent(this, PhoneAudioService::class.java)
                .putExtra(PhoneAudioService.EXTRA_CODE, resultCode)
                .putExtra(PhoneAudioService.EXTRA_DATA, data)
                .putExtra(EXTRA_NODE, node)
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) startForegroundService(start) else startService(start)
        } else {
            PhoneAudio.decline(node)
        }
        finish()
    }

    companion object {
        /** The computer to play on (its node id). */
        const val EXTRA_NODE = "node"
        /** You started it here (else the computer asked). */
        const val EXTRA_OFFER = "offer"
        private const val ASK_MIC = 1
        private const val ASK_CAPTURE = 2
    }
}

/** Captures what apps play and sends it to the computers chosen. */
class PhoneAudioService : Service() {
    private var projection: MediaProjection? = null
    @Volatile
    private var record: AudioRecord? = null
    private var worker: Thread? = null
    /** The computers it plays on (node ids). */
    private val targets = java.util.concurrent.CopyOnWriteArraySet<String>()
    @Volatile
    private var graceUntil = 0L

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        if (intent?.action == ACTION_STOP) {
            stop()
            return START_NOT_STICKY
        }
        if (projection != null) {
            startForeground(PhoneAudio.NOTE_PLAYING, PhoneAudio.playingNote(this, targets.toSet()))
            return START_NOT_STICKY
        }
        PhoneAudio.channel(this)
        val node = intent?.getStringExtra(PhoneAudioConsentActivity.EXTRA_NODE).orEmpty()
        val note = PhoneAudio.playingNote(this, setOfNotNull(node.ifEmpty { null }))
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
            startForeground(PhoneAudio.NOTE_PLAYING, note, ServiceInfo.FOREGROUND_SERVICE_TYPE_MEDIA_PROJECTION)
        } else {
            startForeground(PhoneAudio.NOTE_PLAYING, note)
        }
        val code = intent?.getIntExtra(EXTRA_CODE, 0) ?: 0
        @Suppress("DEPRECATION")
        val data = intent?.getParcelableExtra<Intent>(EXTRA_DATA)
        val mpm = getSystemService(MediaProjectionManager::class.java)
        val mp = if (data != null && node.isNotEmpty() && Build.VERSION.SDK_INT >= AudioPlan.MIN_SDK) {
            runCatching { mpm?.getMediaProjection(code, data) }.getOrNull()
        } else null
        if (mp == null || Build.VERSION.SDK_INT < AudioPlan.MIN_SDK) {
            PhoneAudio.decline(node)
            stop()
            return START_NOT_STICKY
        }
        projection = mp
        current = this
        ResourceLedger.hold(ResourceBook.Kind.SERVICE, "PhoneAudioService")
        graceUntil = System.currentTimeMillis() + START_GRACE_MS
        targets.add(node)
        changed()
        mp.registerCallback(object : MediaProjection.Callback() {
            // Android ended it (Quick Settings, or another capture began).
            override fun onStop() = stop()
        }, Handler(Looper.getMainLooper()))
        val w = Thread({ if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) run(mp, node) }, "phone-audio")
        worker = w
        w.start()
        return START_NOT_STICKY
    }

    fun isTarget(node: String) = node in targets

    /** Play on [node] too; tell it ([offer]: you chose it here, or it asked
     *  while this already played). */
    fun addTarget(node: String, offer: Boolean) {
        if (node.isEmpty()) return
        targets.add(node)
        graceUntil = maxOf(graceUntil, System.currentTimeMillis() + START_GRACE_MS)
        changed()
        if (offer) Thread { offerTo(node) }.start()
    }

    /** [node] stopped listening, or you stopped playing there ([tell] it);
     *  with nobody left, stop. */
    fun targetLeft(node: String, tell: Boolean = false) {
        if (!targets.remove(node)) return
        if (tell) PhoneAudio.decline(node)
        if (targets.isEmpty()) stop() else changed()
    }

    /** Tell [node] this plays to it; with no link yet, connect and try
     *  again for a while. Returns the rate the core gave, if it went. */
    private fun offerTo(node: String): Int? {
        val e = Core.engine() ?: return null
        repeat(OFFER_TRIES) { i ->
            if (current !== this || node !in targets) return null
            val rate = runCatching { e.audioOffer(node, true).toInt() }
                .onFailure { Log.w(TAG, "offer: $it") }.getOrNull()
            if (rate != null) return rate
            if (i == 0) runCatching { e.connect(node) }
            Thread.sleep(OFFER_RETRY_MS)
        }
        return null
    }

    private fun changed() {
        if (current !== this) return
        runCatching {
            getSystemService(NotificationManager::class.java)
                ?.notify(PhoneAudio.NOTE_PLAYING, PhoneAudio.playingNote(this, targets.toSet()))
        }
        PhoneAudio.published(targets.toSet())
    }

    /** The capture loop: 20 ms frames to the computers chosen that listen. */
    @SuppressLint("MissingPermission") // checked before Android's prompt
    @RequiresApi(Build.VERSION_CODES.Q)
    private fun run(mp: MediaProjection, first: String) {
        // The offer says "play" and gives the rate for its link (half
        // through the relay).
        val rate = AudioPlan.rateOf(offerTo(first))
        if (current !== this) return
        val rec = runCatching {
            val config = AudioPlaybackCaptureConfiguration.Builder(mp)
                .addMatchingUsage(AudioAttributes.USAGE_MEDIA)
                .addMatchingUsage(AudioAttributes.USAGE_GAME)
                .addMatchingUsage(AudioAttributes.USAGE_UNKNOWN)
                .build()
            val format = AudioFormat.Builder()
                .setEncoding(AudioFormat.ENCODING_PCM_16BIT)
                .setSampleRate(rate)
                .setChannelMask(AudioFormat.CHANNEL_IN_STEREO)
                .build()
            val min = AudioRecord.getMinBufferSize(rate, AudioFormat.CHANNEL_IN_STEREO, AudioFormat.ENCODING_PCM_16BIT)
            AudioRecord.Builder()
                .setAudioFormat(format)
                .setBufferSizeInBytes(maxOf(min, AudioPlan.frameBytes(rate) * 8))
                .setAudioPlaybackCaptureConfig(config)
                .build()
        }.onFailure { Log.w(TAG, "can't capture: $it") }.getOrNull()
        if (rec == null || rec.state != AudioRecord.STATE_INITIALIZED) {
            rec?.release()
            main.post {
                Toast.makeText(this, getString(R.string.audio_cant_capture), Toast.LENGTH_LONG).show()
                stop()
            }
            return
        }
        record = rec
        if (current !== this) {
            record = null
            rec.release()
            return
        }
        val buf = ByteArray(AudioPlan.frameBytes(rate))
        var lastListened = System.currentTimeMillis()
        var listening = emptyList<String>()
        var checkedAt = 0L
        ResourceLedger.hold(ResourceBook.Kind.AUDIO, "phone-audio capture")
        try {
            rec.startRecording()
            while (current === this) {
                val n = rec.read(buf, 0, buf.size, AudioRecord.READ_BLOCKING)
                if (n < 0) {
                    Log.w(TAG, "capture ended: $n")
                    break
                }
                val engine = Core.engine() ?: break
                val now = System.currentTimeMillis()
                // Who listens: asked twice a second, not every frame.
                if (now - checkedAt >= CHECK_MS) {
                    checkedAt = now
                    listening = engine.audioListeners().filter { it in targets }
                    if (listening.isNotEmpty()) lastListened = now
                    else if (AudioPlan.shouldStop(now, graceUntil, lastListened, IDLE_MS)) break
                }
                if (n == 0 || listening.isEmpty()) continue
                val frame = if (n == buf.size) buf else buf.copyOf(n - n % 4)
                if (frame.isEmpty()) continue
                runCatching { engine.audioFrameTo(frame, rate.toUInt(), AudioPlan.CHANNELS.toUByte(), listening) }
            }
        } catch (e: Exception) {
            Log.w(TAG, "capture: $e")
        } finally {
            // The loop owns the recorder: released here, never under a
            // read, whichever way the loop ended.
            record = null
            runCatching { rec.stop() }
            rec.release()
            ResourceLedger.release(ResourceBook.Kind.AUDIO, "phone-audio capture")
        }
        if (current === this) main.post { stop() }
    }

    private val main = Handler(Looper.getMainLooper())

    private fun stop() {
        val wasCurrent = current === this
        if (wasCurrent) current = null
        targets.toList().forEach { PhoneAudio.decline(it) }
        targets.clear()
        // Ends a read in progress; the capture loop releases it.
        runCatching { record?.stop() }
        runCatching { projection?.stop() }
        projection = null
        worker = null
        if (wasCurrent) PhoneAudio.published(emptySet())
        ResourceLedger.release(ResourceBook.Kind.SERVICE, "PhoneAudioService")
        stopForeground(STOP_FOREGROUND_REMOVE)
        stopSelf()
    }

    override fun onDestroy() {
        if (projection != null || current === this) stop()
        super.onDestroy()
    }

    companion object {
        private const val TAG = "mydomain_audio"
        const val EXTRA_CODE = "code"
        const val EXTRA_DATA = "data"
        const val ACTION_STOP = "stop"
        /** Time for the computer to start listening after the start. */
        private const val START_GRACE_MS = 20_000L
        /** Stop this long after the computers stopped listening (a listen
         *  that isn't renewed runs out after 25 s: a lost link ends here). */
        private const val IDLE_MS = 5_000L
        private const val CHECK_MS = 500L
        private const val OFFER_TRIES = 8
        private const val OFFER_RETRY_MS = 1_500L

        /** The audio playing now, if any. */
        @Volatile
        var current: PhoneAudioService? = null
            private set

        val running: Boolean get() = current != null
    }
}
