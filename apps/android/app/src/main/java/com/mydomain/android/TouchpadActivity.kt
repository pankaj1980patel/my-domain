package com.mydomain.android

import android.content.Context
import android.content.Intent
import android.os.Bundle
import android.view.WindowManager
import android.widget.Toast
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.compose.runtime.CompositionLocalProvider
import com.mydomain.android.core.Core
import com.mydomain.android.ui.FullTouchpad
import com.mydomain.android.ui.LocalToast
import com.mydomain.android.ui.MdTheme
import com.mydomain.android.ui.PREF_TOUCHPAD_SECURE

/**
 * The touchpad full screen (the touchpad tile's Enlarge). Its own screen
 * rather than a dialog, so it sits properly between the status and
 * navigation bars. With the "block screenshots" setting on (the default),
 * the window is secure: on some phones that also stops the three-finger
 * screenshot swipe over it.
 */
class TouchpadActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()
        val node = intent.getStringExtra(EXTRA_NODE) ?: return finish()
        val secure = getSharedPreferences("md", Context.MODE_PRIVATE).getBoolean(PREF_TOUCHPAD_SECURE, true)
        if (secure) window.addFlags(WindowManager.LayoutParams.FLAG_SECURE)
        // In use as a touchpad: don't let the screen time out under you.
        window.addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
        val toast: (String) -> Unit = { Toast.makeText(this, it, Toast.LENGTH_SHORT).show() }
        setContent {
            MdTheme {
                CompositionLocalProvider(LocalToast provides toast) {
                    FullTouchpad(node, onDone = ::finish)
                }
            }
        }
    }

    // On screen counts as the app being visible (see Core.setVisible).
    override fun onStart() {
        super.onStart()
        Core.setVisible(true)
    }

    override fun onStop() {
        super.onStop()
        Core.setVisible(false)
    }

    companion object {
        private const val EXTRA_NODE = "node"

        fun open(ctx: Context, node: String) {
            ctx.startActivity(Intent(ctx, TouchpadActivity::class.java).putExtra(EXTRA_NODE, node))
        }
    }
}
