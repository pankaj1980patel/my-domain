package com.mydomain.android

import android.Manifest
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.net.ConnectivityManager
import android.net.Network
import android.os.Build
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.core.app.ActivityCompat
import androidx.core.content.ContextCompat
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import com.mydomain.android.core.NetworkingUtils.wifiIpv4
import com.mydomain.android.presentation.screens.LoginScreen
import com.mydomain.android.presentation.screens.MainScreen

data class Peer(
    val nodeId: String, val name: String, val ip: String,
    val tcpPort: Int, val udpPort: Int, val wsPort: Int, val source: String,
)

data class DevCredentials(val server: String, val user: String, val pass: String, val key: String)

data class LogEntry(val dir: String, val peer: String, val ip: String, val protocol: String, val text: String, val ok: Boolean)

class MainActivity : ComponentActivity() {
    private var connectivityManager: ConnectivityManager? = null
    private var netCallback: ConnectivityManager.NetworkCallback? = null

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()

        // Ask for READ_PHONE_STATE so a later launch can use the IMEI as the
        // device id (this launch falls back to ANDROID_ID — the service, which
        // resolves the id, starts immediately below and the id is set once).
        if (ContextCompat.checkSelfPermission(this, Manifest.permission.READ_PHONE_STATE)
            != PackageManager.PERMISSION_GRANTED
        ) {
            runCatching {
                ActivityCompat.requestPermissions(
                    this, arrayOf(Manifest.permission.READ_PHONE_STATE), 1001
                )
            }
        }

        // Start the background service for networking and clipboard sync
        val serviceIntent = Intent(this, NetService::class.java)
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            startForegroundService(serviceIntent)
        } else {
            startService(serviceIntent)
        }

        // Re-register on network change.
        connectivityManager = getSystemService(Context.CONNECTIVITY_SERVICE) as ConnectivityManager
        netCallback = object : ConnectivityManager.NetworkCallback() {
            override fun onAvailable(network: Network) { RustNet.nativeNetworkChanged(wifiIpv4()) }
            override fun onLost(network: Network) { RustNet.nativeNetworkChanged(wifiIpv4()) }
        }
        runCatching { connectivityManager?.registerDefaultNetworkCallback(netCallback!!) }

        val prefs = getSharedPreferences("md", Context.MODE_PRIVATE)
        // Device lab: log in with the test account in the background, so it
        // works even when the phone is locked (Compose only runs on screen).
        devCredentials(intent)?.let { dev -> Thread { devLogin(prefs, dev) }.start() }
        setContent { MaterialTheme { App(prefs) } }
    }

    /** Device-lab test account from launch extras (scripts/lab/android.sh
     *  passes md_server/md_user/md_pass/md_key). Debuggable builds only. */
    private fun devCredentials(intent: Intent?): DevCredentials? {
        if (applicationInfo.flags and android.content.pm.ApplicationInfo.FLAG_DEBUGGABLE == 0) return null
        val get = { k: String -> intent?.getStringExtra(k)?.takeIf { it.isNotBlank() } }
        return DevCredentials(
            get("md_server") ?: return null, get("md_user") ?: return null,
            get("md_pass") ?: return null, get("md_key") ?: return null,
        )
    }

    private fun devLogin(prefs: android.content.SharedPreferences, dev: DevCredentials) {
        // NetService starts the engine; on a fresh launch it may not be up yet.
        var err = RustNet.nativeAuthLogin(dev.server, dev.user, dev.pass)
        repeat(100) {
            if (err == "ERROR: not started") {
                Thread.sleep(100); err = RustNet.nativeAuthLogin(dev.server, dev.user, dev.pass)
            }
        }
        if (err.isEmpty()) err = RustNet.nativeSetEncryptionKey(dev.key)
        if (err.isEmpty()) {
            prefs.edit().putString("server", dev.server).putString("user", dev.user).apply()
            RustNet.nativeRefresh()
        }
        android.util.Log.i("mydomain_net", "lab auto-login: ${if (err.isEmpty()) "ok" else err}")
    }

    override fun onDestroy() {
        super.onDestroy()
        netCallback?.let { runCatching { connectivityManager?.unregisterNetworkCallback(it) } }
    }
}

@Composable
fun App(prefs: android.content.SharedPreferences) {
    var ready by remember { mutableStateOf(RustNet.nativeIsReady() == "true") }
    // A background login (device lab) can finish while the login screen shows.
    LaunchedEffect(ready) {
        while (!ready) {
            kotlinx.coroutines.delay(500)
            ready = RustNet.nativeIsReady() == "true"
        }
    }
    if (!ready) {
        LoginScreen(prefs) { ready = true }
    } else {
        MainScreen(prefs, onLogout = { ready = false })
    }
}
