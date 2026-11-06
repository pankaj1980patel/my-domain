package com.mydomain.android

import android.app.Activity
import android.app.AlertDialog
import android.app.Notification
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.media.AudioAttributes
import android.media.AudioFormat
import android.media.AudioTrack
import android.net.wifi.WifiManager
import android.os.Build
import android.os.Bundle
import android.os.Handler
import android.os.IBinder
import android.os.Looper
import android.os.PowerManager
import android.os.Process
import android.util.Log
import android.widget.Toast
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateMapOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.core.app.NotificationCompat
import com.mydomain.android.core.AudioJitter
import com.mydomain.android.core.AudioRoutes
import com.mydomain.android.core.Core
import com.mydomain.android.core.PlaybackPlan
import com.mydomain.android.core.friendlyError
import com.mydomain.android.ffi.MdException
import org.json.JSONObject

/**
 * A computer's sound played on this phone, and the earbuds on it (mdcore
 * `features::audio_share`, the other way round from PhoneAudio.kt: here
 * the computer sends, this phone plays). Two ways in, and both wait for a
 * tap here:
 *
 *  - "Play <computer>'s sound here" on the computer's page ([playHere]);
 *  - "Play on <phone>" on the computer: its offer arrives ([onOffer]) and
 *    the phone asks (a prompt on screen, else a notification with Play and
 *    Not now). Nothing plays without the tap.
 *
 * While it plays, a foreground service of type mediaPlayback holds an
 * ongoing notification "Playing <computer>'s sound" with Stop; it renews
 * the listen every 10 s (so the computer stops by itself if this phone
 * goes away) and plays the frames through an AudioTrack (media usage)
 * behind a small jitter buffer ([AudioJitter]). It stops on Stop (here,
 * the notification, the computer), when the link goes, or when the frames
 * stop coming. One computer at a time.
 *
 * The playback is marked not capturable, so this phone's own audio sent
 * to a computer (PhoneAudio.kt) never carries it back.
 */
object ComputerAudio {
    private const val TAG = "mydomain_caudio"
    const val NOTE_ASK = 7321
    const val NOTE_PLAYING = 7322

    /** The computer whose sound plays here (or was asked to), if any. */
    var playingFrom by mutableStateOf<String?>(null)
        private set
    /** Frames are coming and playing (else waiting for the first, or
     *  catching up). */
    var flowing by mutableStateOf(false)
        private set
    /** Computers that offered their sound, waiting for a tap here. */
    val offers = mutableStateMapOf<String, Long>()
    /** Why it last stopped by itself, for the tile. */
    var lastStop by mutableStateOf<String?>(null)
        private set

    private val main = Handler(Looper.getMainLooper())

    internal fun published(node: String?, flowing: Boolean, why: String? = null) {
        main.post {
            playingFrom = node
            this.flowing = flowing
            if (node != null) lastStop = null else if (why != null) lastStop = why
        }
    }

    /** A computer offered its sound ([on]), or stopped (also its "no"). */
    fun onOffer(ctx: Context, from: String, on: Boolean) {
        val app = ctx.applicationContext
        if (!on) {
            main.post { offers.remove(from) }
            cancelAsk(app)
            ComputerAudioService.current?.sourceStopped(from)
            return
        }
        val svc = ComputerAudioService.current
        // It says "playing" after this phone's listen: not a new offer.
        if (svc != null && svc.node == from) return
        main.post { offers[from] = System.currentTimeMillis() }
        PhoneAudio.channel(app)
        val ask = askIntent(app, from, ComputerAudioAskActivity.ASK)
        if (Core.visible && runCatching { app.startActivity(ask) }.isSuccess) return
        val name = Core.peerName(from)
        val n = NotificationCompat.Builder(app, PhoneAudio.CH)
            .setSmallIcon(android.R.drawable.ic_lock_silent_mode_off)
            .setContentTitle(app.getString(R.string.caudio_ask_title, name))
            .setContentText(app.getString(R.string.caudio_ask_body))
            .setPriority(NotificationCompat.PRIORITY_HIGH)
            .setAutoCancel(true)
            .setContentIntent(pending(app, 0, ask))
            .addAction(0, app.getString(R.string.caudio_play), pending(app, 1, askIntent(app, from, ComputerAudioAskActivity.PLAY)))
            .addAction(0, app.getString(R.string.caudio_not_now), pending(app, 2, askIntent(app, from, ComputerAudioAskActivity.DECLINE)))
            .build()
        runCatching { app.getSystemService(NotificationManager::class.java)?.notify(NOTE_ASK, n) }
            .onFailure { Log.w(TAG, "can't ask: $it") }
    }

    private fun askIntent(ctx: Context, node: String, what: String) =
        Intent(ctx, ComputerAudioAskActivity::class.java)
            .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
            .putExtra(ComputerAudioAskActivity.EXTRA_NODE, node)
            .putExtra(ComputerAudioAskActivity.EXTRA_WHAT, what)

    private fun pending(ctx: Context, code: Int, i: Intent) =
        PendingIntent.getActivity(ctx, NOTE_ASK + code, i, PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)

    fun cancelAsk(ctx: Context) {
        runCatching { ctx.getSystemService(NotificationManager::class.java)?.cancel(NOTE_ASK) }
    }

    /** You chose to play computer [node]'s sound here ([ctx]: an activity,
     *  or the app). */
    fun playHere(ctx: Context, node: String) {
        if (node.isEmpty()) return
        cancelAsk(ctx)
        main.post { offers.remove(node) }
        val start = Intent(ctx, ComputerAudioService::class.java)
            .putExtra(ComputerAudioAskActivity.EXTRA_NODE, node)
        runCatching {
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) ctx.startForegroundService(start) else ctx.startService(start)
        }.onFailure { Log.w(TAG, "can't start: $it") }
    }

    /** "Not now" to computer [node]'s offer: it stops waiting. */
    fun decline(ctx: Context, node: String) {
        cancelAsk(ctx)
        main.post { offers.remove(node) }
        Thread { runCatching { Core.engine()?.audioListen(node, false) } }.start()
    }

    /** Stop playing (telling the computer). */
    fun stop() {
        ComputerAudioService.current?.stop(tell = true)
    }

    /** A frame from a computer (the core's thread; many a second; the
     *  bytes as they came, no JSON): into the buffer if it's the one
     *  playing here. Nothing else happens on this thread. */
    fun onFrame(from: String, seq: Long, rate: Int, channels: Int, pcm: ByteArray) {
        val svc = ComputerAudioService.current ?: return
        if (from != svc.node) return
        svc.push(seq, rate, channels, pcm)
    }

    /** The stream with [node] moved to (or off) a nearby local link: a
     *  smaller buffer floor, and the output's low-latency mode. */
    fun onNearby(node: String, lowLatency: Boolean) {
        val svc = ComputerAudioService.current ?: return
        if (node == svc.node) svc.setLowLatency(lowLatency)
    }

    /** What the tile's details show while it plays. */
    data class PlayStats(
        val bufferedMs: Int,
        val targetMs: Int,
        val underruns: Int,
        val concealedMs: Int,
        val late: Int,
        val recovered: Long,
        val outputUnderruns: Int,
        val driftPpm: Int,
        val lowLatency: Boolean,
        /** Lost frames Opus made up (its packet-loss concealment). */
        val plc: Long = 0,
        /** Coming over Bluetooth now (the buffer keeps more). */
        val viaBluetooth: Boolean = false,
    )

    /** The details of what plays now (null: nothing). */
    var stats by mutableStateOf<PlayStats?>(null)
        private set

    internal fun publishStats(s: PlayStats?) {
        main.post { stats = s }
    }

    /** The link to [node] went: if it played here, that's the end. */
    fun linkLost(node: String) {
        ComputerAudioService.current?.linkLost(node)
    }

    /** "Playing <computer>'s sound", with Stop. */
    fun playingNote(ctx: Context, node: String): Notification {
        val stop = PendingIntent.getService(
            ctx, 0, Intent(ctx, ComputerAudioService::class.java).setAction(ComputerAudioService.ACTION_STOP),
            PendingIntent.FLAG_IMMUTABLE,
        )
        val body = ctx.getString(R.string.caudio_playing_body)
        return NotificationCompat.Builder(ctx, PhoneAudio.CH)
            .setSmallIcon(android.R.drawable.ic_lock_silent_mode_off)
            .setContentTitle(ctx.getString(R.string.caudio_playing_title, Core.peerName(node)))
            .setContentText(body)
            .setStyle(NotificationCompat.BigTextStyle().bigText(body))
            .setOngoing(true)
            .setOnlyAlertOnce(true)
            .addAction(0, ctx.getString(R.string.audio_stop), stop)
            .build()
    }
}

/** A computer offers its sound: Play or Not now (a dialog over whatever
 *  shows), or the notification's Play / Not now straight through. */
class ComputerAudioAskActivity : Activity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        ComputerAudio.cancelAsk(this)
        val node = intent?.getStringExtra(EXTRA_NODE).orEmpty()
        if (node.isEmpty()) return finish()
        when (intent?.getStringExtra(EXTRA_WHAT)) {
            PLAY -> {
                ComputerAudio.playHere(this, node)
                return finish()
            }
            DECLINE -> {
                ComputerAudio.decline(this, node)
                return finish()
            }
        }
        val name = Core.peerName(node)
        AlertDialog.Builder(this, android.R.style.Theme_DeviceDefault_Dialog_Alert)
            .setTitle(getString(R.string.caudio_ask_title, name))
            .setMessage(getString(R.string.caudio_ask_body))
            .setPositiveButton(R.string.caudio_play) { _, _ ->
                ComputerAudio.playHere(this, node)
                finish()
            }
            .setNegativeButton(R.string.caudio_not_now) { _, _ ->
                ComputerAudio.decline(this, node)
                finish()
            }
            // Dismissed: no answer yet; the offer stays on its page here.
            .setOnCancelListener { finish() }
            .show()
    }

    companion object {
        const val EXTRA_NODE = "node"
        const val EXTRA_WHAT = "what"
        const val ASK = "ask"
        const val PLAY = "play"
        const val DECLINE = "decline"
    }
}

/** Plays the computer's sound: renews the listen, buffers, plays. */
class ComputerAudioService : Service() {
    /** The computer playing here (node id). */
    @Volatile
    var node = ""
        private set
    private val jitter = AudioJitter()
    @Volatile
    private var lastFrame = 0L
    @Volatile
    private var startedAt = 0L
    @Volatile
    private var running = false
    private val main = Handler(Looper.getMainLooper())

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        if (intent?.action == ACTION_STOP) {
            stop(tell = true)
            return START_NOT_STICKY
        }
        val n = intent?.getStringExtra(ComputerAudioAskActivity.EXTRA_NODE).orEmpty()
        PhoneAudio.channel(this)
        val note = ComputerAudio.playingNote(this, n.ifEmpty { node })
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
            startForeground(ComputerAudio.NOTE_PLAYING, note, ServiceInfo.FOREGROUND_SERVICE_TYPE_MEDIA_PLAYBACK)
        } else {
            startForeground(ComputerAudio.NOTE_PLAYING, note)
        }
        if (n.isEmpty()) {
            if (current !== this) stop(tell = false)
            return START_NOT_STICKY
        }
        if (current === this && n == node) return START_NOT_STICKY
        // Another computer: that one stops.
        val old = node
        if (current === this && old.isNotEmpty()) {
            Thread { runCatching { Core.engine()?.audioListen(old, false) } }.start()
        }
        node = n
        jitter.reset()
        setLowLatency(Core.nearby[n]?.lowLatency == true)
        lastFrame = 0L
        startedAt = System.currentTimeMillis()
        current = this
        holdLocks()
        ComputerAudio.published(n, false)
        if (!running) {
            running = true
            Thread({ renew() }, "computer-audio-listen").start()
            Thread({ play() }, "computer-audio-play").start()
        }
        return START_NOT_STICKY
    }

    fun push(seq: Long, rate: Int, channels: Int, pcm: ByteArray) {
        jitter.push(seq, rate, channels, pcm)
        val first = lastFrame == 0L
        lastFrame = System.currentTimeMillis()
        if (first) ComputerAudio.published(node, true)
    }

    /** On a nearby local link: a lower buffer floor, and the output opened
     *  in low-latency mode (the player reopens it). */
    fun setLowLatency(on: Boolean) {
        jitter.setLowLatency(on)
        lowLatency = on
    }

    @Volatile
    private var lowLatency = false
    @Volatile
    private var outputUnderruns = 0
    private var wifiLock: WifiManager.WifiLock? = null
    private var wakeLock: PowerManager.WakeLock? = null

    /** Keep the radio awake (Wi-Fi power save batches packets: stutter)
     *  and the CPU running with the screen off, only while playing. */
    @Suppress("DEPRECATION", "InlinedApi")
    private fun holdLocks() {
        if (wifiLock == null) {
            val mode = when (PlaybackPlan.wifiLock(Build.VERSION.SDK_INT)) {
                PlaybackPlan.WifiLock.LowLatency -> WifiManager.WIFI_MODE_FULL_LOW_LATENCY
                PlaybackPlan.WifiLock.HighPerf -> WifiManager.WIFI_MODE_FULL_HIGH_PERF
            }
            wifiLock = runCatching {
                applicationContext.getSystemService(WifiManager::class.java)
                    ?.createWifiLock(mode, "mydomain:computer-audio")
                    ?.apply { setReferenceCounted(false); acquire() }
            }.onFailure { Log.w(TAG, "no Wi-Fi lock: $it") }.getOrNull()
        }
        if (wakeLock == null) {
            wakeLock = runCatching {
                getSystemService(PowerManager::class.java)
                    ?.newWakeLock(PowerManager.PARTIAL_WAKE_LOCK, "mydomain:computer-audio")
                    ?.apply { setReferenceCounted(false); acquire() }
            }.onFailure { Log.w(TAG, "no wake lock: $it") }.getOrNull()
        }
    }

    private fun releaseLocks() {
        runCatching { wifiLock?.takeIf { it.isHeld }?.release() }
        wifiLock = null
        runCatching { wakeLock?.takeIf { it.isHeld }?.release() }
        wakeLock = null
    }

    private fun publishStats() {
        val n = node
        if (n.isEmpty()) return ComputerAudio.publishStats(null)
        val rx = runCatching { JSONObject(Core.engine()?.audioRxStats(n) ?: "{}") }
            .getOrDefault(JSONObject())
        val recovered = rx.optLong("recovered")
        val viaBle = AudioRoutes.viaBluetooth(runCatching { Core.engine()?.audioVia(n) }.getOrNull())
        // Over Bluetooth the buffer keeps more (records come 60 ms at a time).
        if (viaBle != jitter.bluetooth()) {
            if (viaBle) jitter.setBluetooth(true) else jitter.setLowLatency(lowLatency)
        }
        val s = jitter.stats()
        ComputerAudio.publishStats(
            ComputerAudio.PlayStats(
                bufferedMs = s.bufferedMs,
                targetMs = s.targetMs,
                underruns = s.underruns,
                concealedMs = s.concealedMs,
                late = s.late,
                recovered = recovered,
                outputUnderruns = outputUnderruns,
                driftPpm = s.driftPpm,
                lowLatency = jitter.lowLatency(),
                plc = rx.optLong("concealed"),
                viaBluetooth = viaBle,
            ),
        )
    }

    /** Renew the listen every 10 s (connecting first if needed), and stop
     *  when the frames stop coming. */
    private fun renew() {
        var renewedAt = 0L
        var tries = 0
        while (running && current === this) {
            val now = System.currentTimeMillis()
            val n = node
            if (now - renewedAt >= RENEW_MS) {
                val e = Core.engine()
                val r = if (e == null) null else runCatching { e.audioListen(n, true) }
                when {
                    r == null -> {}
                    r.isSuccess -> {
                        renewedAt = now
                        tries = 0
                    }
                    r.exceptionOrNull() is MdException.NotConnected && tries < TRIES -> {
                        // No link yet: the core started connecting; again soon.
                        tries++
                        renewedAt = now - RENEW_MS + RETRY_MS
                    }
                    else -> {
                        val why = r.exceptionOrNull()?.let { friendlyError(it) }
                            ?: getString(R.string.caudio_no_frames, Core.peerName(n))
                        Log.w(TAG, "listen: $why")
                        main.post { stopSaying(why) }
                        return
                    }
                }
            }
            val last = lastFrame
            if (last == 0L && now - startedAt > NO_ANSWER_MS) {
                main.post { stopSaying(getString(R.string.caudio_no_frames, Core.peerName(n))) }
                return
            }
            if (last != 0L && now - last > STALL_MS) {
                main.post { stopSaying(getString(R.string.caudio_stalled, Core.peerName(n))) }
                return
            }
            publishStats()
            Thread.sleep(CHECK_MS)
        }
    }

    /** The player, on its own urgent-audio thread (frames arrive on the
     *  core's thread and only go into the buffer): [PlaybackPlan.PERIOD_MS]
     *  at a time from the buffer to the output at the stream's rate
     *  (silence while it fills); the blocking write keeps the pace. The
     *  output is reopened when the rate or the nearby mode changes. One
     *  buffer, reused: nothing is allocated per write. */
    private fun play() {
        Process.setThreadPriority(Process.THREAD_PRIORITY_URGENT_AUDIO)
        var track: AudioTrack? = null
        var opened: PlaybackPlan.Track? = null
        var underrunBase = 0
        var buf = ShortArray(0)
        try {
            while (running && current === this) {
                val wanted = PlaybackPlan.trackFor(jitter.rate, lowLatency, Build.VERSION.SDK_INT)
                if (wanted.rate == 0) {
                    Thread.sleep(10)
                    continue
                }
                if (PlaybackPlan.reopen(opened, wanted)) {
                    track?.let { runCatching { it.stop() }; it.release() }
                    track = openTrack(wanted)
                    opened = wanted
                    if (track == null) {
                        main.post { stopSaying(getString(R.string.caudio_cant_play)) }
                        return
                    }
                    underrunBase = underrunsOf(track)
                    track.play()
                }
                val frames = PlaybackPlan.periodFrames(wanted.rate)
                if (buf.size != frames * 2) buf = ShortArray(frames * 2)
                jitter.pull(buf, frames)
                val t = track ?: break
                if (t.write(buf, 0, buf.size) < 0) {
                    Log.w(TAG, "the output went")
                    break
                }
                outputUnderruns = PlaybackPlan.underruns(underrunBase, underrunsOf(t))
            }
        } catch (e: Exception) {
            Log.w(TAG, "player: $e")
        } finally {
            track?.let { runCatching { it.stop() }; it.release() }
        }
    }

    private fun underrunsOf(t: AudioTrack): Int =
        if (Build.VERSION.SDK_INT >= PlaybackPlan.SDK_UNDERRUNS) t.underrunCount else -1

    private fun openTrack(wanted: PlaybackPlan.Track): AudioTrack? = runCatching {
        val rate = wanted.rate
        val attrs = AudioAttributes.Builder()
            .setUsage(AudioAttributes.USAGE_MEDIA)
            .setContentType(AudioAttributes.CONTENT_TYPE_MUSIC)
        // Never captured: this phone's own audio on a computer (PhoneAudio)
        // must not carry the computer's sound back to it.
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
            attrs.setAllowedCapturePolicy(AudioAttributes.ALLOW_CAPTURE_BY_NONE)
        }
        val format = AudioFormat.Builder()
            .setEncoding(AudioFormat.ENCODING_PCM_16BIT)
            .setSampleRate(rate)
            .setChannelMask(AudioFormat.CHANNEL_OUT_STEREO)
            .build()
        val min = AudioTrack.getMinBufferSize(rate, AudioFormat.CHANNEL_OUT_STEREO, AudioFormat.ENCODING_PCM_16BIT)
        // A few times the minimum: room for a hiccup of this thread; the
        // jitter buffer does the real smoothing.
        val b = AudioTrack.Builder()
            .setAudioAttributes(attrs.build())
            .setAudioFormat(format)
            .setBufferSizeInBytes(PlaybackPlan.bufferBytes(min, rate))
            .setTransferMode(AudioTrack.MODE_STREAM)
        if (wanted.lowLatency && Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            b.setPerformanceMode(AudioTrack.PERFORMANCE_MODE_LOW_LATENCY)
        }
        b.build().takeIf { it.state == AudioTrack.STATE_INITIALIZED }
    }.onFailure { Log.w(TAG, "can't play: $it") }.getOrNull()

    /** The computer stopped sending (or said no to this phone's listen). */
    fun sourceStopped(from: String) {
        if (from != node) return
        val name = Core.peerName(from)
        val why = if (lastFrame == 0L) getString(R.string.caudio_refused, name) else getString(R.string.caudio_stopped_by, name)
        main.post { stopSaying(why, tell = false) }
    }

    fun linkLost(n: String) {
        if (n != node) return
        main.post { stopSaying(getString(R.string.caudio_link_lost, Core.peerName(n)), tell = false) }
    }

    private fun stopSaying(why: String, tell: Boolean = true) {
        if (current !== this) return
        Toast.makeText(this, why, Toast.LENGTH_LONG).show()
        stop(tell, why)
    }

    fun stop(tell: Boolean, why: String? = null) {
        val wasCurrent = current === this
        if (wasCurrent) current = null
        running = false
        val n = node
        if (tell && n.isNotEmpty()) Thread { runCatching { Core.engine()?.audioListen(n, false) } }.start()
        if (wasCurrent) ComputerAudio.published(null, false, why)
        releaseLocks()
        ComputerAudio.publishStats(null)
        stopForeground(STOP_FOREGROUND_REMOVE)
        stopSelf()
    }

    override fun onDestroy() {
        if (current === this) stop(tell = true)
        running = false
        releaseLocks()
        super.onDestroy()
    }

    companion object {
        private const val TAG = "mydomain_caudio"
        const val ACTION_STOP = "stop"
        /** The core's listen lasts 25 s; renewed well before. */
        private const val RENEW_MS = 10_000L
        private const val RETRY_MS = 1_500L
        private const val TRIES = 8
        /** The computer never started (its capture was refused, say). */
        private const val NO_ANSWER_MS = 45_000L
        /** Nothing for this long while playing: it went away. */
        private const val STALL_MS = 6_000L
        private const val CHECK_MS = 500L

        /** The computer's sound playing now, if any. */
        @Volatile
        var current: ComputerAudioService? = null
            private set
    }
}
