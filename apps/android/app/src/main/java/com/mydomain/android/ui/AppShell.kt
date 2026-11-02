package com.mydomain.android.ui

import android.content.SharedPreferences
import androidx.activity.compose.BackHandler
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Home
import androidx.compose.material.icons.filled.List
import androidx.compose.material.icons.filled.Settings
import androidx.compose.material3.Icon
import androidx.compose.material3.NavigationBar
import androidx.compose.material3.NavigationBarItem
import androidx.compose.material3.NavigationBarItemDefaults
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.unit.dp
import com.mydomain.android.core.Core
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch

/** Show a short message at the bottom of the screen. */
val LocalToast = staticCompositionLocalOf<(String) -> Unit> { {} }

private enum class Tab(val label: String, val icon: ImageVector) {
    Devices("Devices", Icons.Filled.Home),
    Activity("Activity", Icons.Filled.List),
    Settings("Settings", Icons.Filled.Settings),
}

/** Sign-in until ready; then tabs (Devices, Activity, Settings) plus the
 *  pushed Device and Developer screens. */
@Composable
fun App(prefs: SharedPreferences) {
    val snack = remember { SnackbarHostState() }
    val scope = rememberCoroutineScope()
    val toast: (String) -> Unit = { msg -> scope.launch { snack.currentSnackbarData?.dismiss(); snack.showSnackbar(msg) } }

    // A background sign-in (device lab) can finish while sign-in shows.
    LaunchedEffect(Core.ready) {
        while (!Core.ready) {
            Core.reload(); delay(500)
        }
    }

    CompositionLocalProvider(LocalToast provides toast) {
        if (!Core.ready) {
            Scaffold(snackbarHost = { SnackbarHost(snack) }, containerColor = Color.Transparent) { inner ->
                Box(Modifier.padding(inner)) { SignInScreen(prefs) }
            }
        } else {
            Main(prefs, snack)
        }
    }
}

@Composable
private fun Main(prefs: SharedPreferences, snack: SnackbarHostState) {
    var tab by rememberSaveable { mutableStateOf(Tab.Devices) }
    var device by rememberSaveable { mutableStateOf<String?>(null) }
    var developer by rememberSaveable { mutableStateOf(false) }

    BackHandler(enabled = device != null || developer || tab != Tab.Devices) {
        when {
            device != null -> device = null
            developer -> developer = false
            else -> tab = Tab.Devices
        }
    }

    Scaffold(
        containerColor = Color.Transparent,
        snackbarHost = { SnackbarHost(snack) },
        bottomBar = {
            if (device == null && !developer) {
                NavigationBar(containerColor = SpaceBar, tonalElevation = 0.dp) {
                    Tab.entries.forEach { t ->
                        NavigationBarItem(
                            selected = tab == t,
                            onClick = { tab = t },
                            icon = { Icon(t.icon, contentDescription = null) },
                            label = { Text(t.label) },
                            colors = NavigationBarItemDefaults.colors(
                                selectedIconColor = T.text, selectedTextColor = T.text,
                                indicatorColor = T.surface2,
                                unselectedIconColor = T.text3, unselectedTextColor = T.text3,
                            ),
                        )
                    }
                }
            }
        },
    ) { inner ->
        Box(Modifier.fillMaxSize().padding(inner)) {
            when {
                device != null -> DeviceScreen(device!!, onBack = { device = null })
                developer -> DeveloperScreen(onBack = { developer = false })
                tab == Tab.Devices -> HomeScreen(onOpen = { device = it })
                tab == Tab.Activity -> ActivityScreen()
                else -> SettingsScreen(prefs, onDeveloper = { developer = true })
            }
        }
    }
}
