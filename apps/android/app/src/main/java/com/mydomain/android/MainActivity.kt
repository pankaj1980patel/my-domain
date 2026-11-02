package com.mydomain.android

import android.Manifest
import android.content.Context
import android.content.Intent
import android.content.SharedPreferences
import android.content.pm.ApplicationInfo
import android.content.pm.PackageManager
import android.os.Build
import android.os.Bundle
import android.util.Log
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.core.app.ActivityCompat
import androidx.core.content.ContextCompat
import com.mydomain.android.core.Core
import com.mydomain.android.ui.App
import com.mydomain.android.ui.MdTheme

data class DevCredentials(val server: String, val user: String, val pass: String, val key: String)

class MainActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()

        // Ask for READ_PHONE_STATE so a later launch can use the IMEI as the
        // device id (this launch falls back to ANDROID_ID: the service, which
        // resolves the id, starts immediately below and the id is set once).
        val wanted = buildList {
            add(Manifest.permission.READ_PHONE_STATE)
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) add(Manifest.permission.POST_NOTIFICATIONS)
        }.filter { ContextCompat.checkSelfPermission(this, it) != PackageManager.PERMISSION_GRANTED }
        if (wanted.isNotEmpty()) runCatching { ActivityCompat.requestPermissions(this, wanted.toTypedArray(), 1001) }

        // The service owns the engine (networking, clipboard, notifications).
        val serviceIntent = Intent(this, NetService::class.java)
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) startForegroundService(serviceIntent) else startService(serviceIntent)

        val prefs = getSharedPreferences("md", Context.MODE_PRIVATE)
        // Device lab: sign in with the test account in the background, so it
        // works even when the phone is locked (Compose only runs on screen).
        devCredentials(intent)?.let { dev -> Thread { devLogin(prefs, dev) }.start() }
        setContent { MdTheme { App(prefs) } }
    }

    override fun onResume() {
        super.onResume()
        Thread { Core.reload() }.start()
    }

    /** Device-lab test account from launch extras (scripts/lab/android.sh
     *  passes md_server/md_user/md_pass/md_key). Debuggable builds only. */
    private fun devCredentials(intent: Intent?): DevCredentials? {
        if (applicationInfo.flags and ApplicationInfo.FLAG_DEBUGGABLE == 0) return null
        val get = { k: String -> intent?.getStringExtra(k)?.takeIf { it.isNotBlank() } }
        return DevCredentials(
            get("md_server") ?: return null, get("md_user") ?: return null,
            get("md_pass") ?: return null, get("md_key") ?: return null,
        )
    }

    private fun devLogin(prefs: SharedPreferences, dev: DevCredentials) {
        val result = runCatching {
            kotlinx.coroutines.runBlocking { Core.signIn(dev.server, dev.user, dev.pass, dev.key, create = false) }
        }
        result.onSuccess { prefs.edit().putString("server", dev.server).putString("user", dev.user).apply() }
        Log.i("mydomain_net", "lab auto-login: ${result.exceptionOrNull()?.message ?: "ok"}")
    }
}
