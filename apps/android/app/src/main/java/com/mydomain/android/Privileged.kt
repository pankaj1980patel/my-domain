package com.mydomain.android

import com.mydomain.android.core.NotificationIds
import android.content.pm.PackageManager
import android.os.Build
import android.util.Log
import com.mydomain.android.core.Core
import rikka.shizuku.Shizuku
import java.util.concurrent.TimeUnit

/**
 * Shell commands with ADB rights, through Shizuku (ADR-0006): what lets
 * [Radios] switch Wi-Fi, Bluetooth, airplane mode and mobile data with no
 * tap on stock phones. All Shizuku calls live here.
 *
 * Shizuku is a separate app you start once (wireless debugging) and again
 * after each reboot; this app then asks for its permission once. Until
 * both, [state] says what's missing and [Radios] opens settings instead.
 */
object Privileged {
    private const val TAG = "mydomain_shizuku"
    const val SHIZUKU_PACKAGE = "moe.shizuku.privileged.api"
    private const val ASK = NotificationIds.PRIVILEGED_ASK

    /** Shizuku's state, for the phone's Settings and the status. */
    enum class State { NOT_INSTALLED, NOT_RUNNING, NOT_ALLOWED, READY }

    /** Called when the state may have changed (Shizuku started or stopped,
     *  the permission answered). */
    var onChange: () -> Unit = {}
    private var watching = false

    fun init() {
        if (watching) return
        watching = true
        runCatching {
            Shizuku.addBinderReceivedListenerSticky { onChange() }
            Shizuku.addBinderDeadListener { onChange() }
            Shizuku.addRequestPermissionResultListener { _: Int, _: Int -> onChange() }
        }.onFailure { Log.w(TAG, "can't watch Shizuku: $it") }
    }

    fun state(): State {
        val running = runCatching { Shizuku.pingBinder() }.getOrDefault(false)
        if (!running) {
            val installed = runCatching { Core.app.packageManager.getPackageInfo(SHIZUKU_PACKAGE, 0) }.isSuccess
            return if (installed) State.NOT_RUNNING else State.NOT_INSTALLED
        }
        val allowed = runCatching {
            !Shizuku.isPreV11() && Shizuku.checkSelfPermission() == PackageManager.PERMISSION_GRANTED
        }.getOrDefault(false)
        return if (allowed) State.READY else State.NOT_ALLOWED
    }

    fun ready() = state() == State.READY

    /** Ask for Shizuku's permission (its own prompt). Only when running. */
    fun ask() {
        runCatching { if (Shizuku.pingBinder()) Shizuku.requestPermission(ASK) }
            .onFailure { Log.w(TAG, "can't ask: $it") }
    }

    /** Run [cmd] as the shell user; true if it exited 0 within 10 s. */
    fun run(vararg cmd: String): Boolean {
        if (!ready()) return false
        return runCatching {
            // Shizuku 13 keeps newProcess private (it wants a bound
            // "user service" instead); for a one-line command it's fine.
            val m = Shizuku::class.java.getDeclaredMethod(
                "newProcess", Array<String>::class.java, Array<String>::class.java, String::class.java,
            )
            m.isAccessible = true
            val p = m.invoke(null, arrayOf(*cmd), null, null) as Process
            val done = p.waitFor(10, TimeUnit.SECONDS)
            if (!done) p.destroy()
            val code = if (done) p.exitValue() else -1
            if (code != 0) {
                val err = runCatching { p.errorStream.bufferedReader().readText().take(200) }.getOrDefault("")
                Log.w(TAG, "${cmd.joinToString(" ")} → $code $err")
            }
            code == 0
        }.onFailure { Log.w(TAG, "${cmd.joinToString(" ")} failed: $it") }.getOrDefault(false)
    }

    /** The command that switches [what] (as in [Radios.KINDS]), if any. */
    fun radioCommand(what: String, on: Boolean): Array<String>? {
        val onOff = if (on) "enable" else "disable"
        return when (what) {
            "wifi" -> arrayOf("svc", "wifi", onOff)
            "bluetooth" -> arrayOf("svc", "bluetooth", onOff)
            "data" -> arrayOf("svc", "data", onOff)
            "airplane" -> if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) arrayOf("cmd", "connectivity", "airplane-mode", onOff) else null
            "location" -> if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) arrayOf("cmd", "location", "set-location-enabled", on.toString()) else null
            // A hotspot needs a network name and password: left to Settings.
            else -> null
        }
    }
}
