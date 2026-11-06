package com.mydomain.android.ui

import android.content.SharedPreferences
import androidx.activity.compose.BackHandler
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.annotation.StringRes
import androidx.compose.foundation.layout.padding
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Edit
import androidx.compose.material.icons.filled.Home
import androidx.compose.material.icons.filled.List
import androidx.compose.material.icons.filled.Settings
import androidx.compose.material3.Icon
import androidx.compose.material3.NavigationBar
import androidx.compose.material3.NavigationBarItem
import androidx.compose.material3.NavigationBarItemDefaults
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SnackbarDuration
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.SnackbarResult
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
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import com.mydomain.android.R
import com.mydomain.android.core.Core
import com.mydomain.android.core.Findable
import com.mydomain.android.core.FoundKind
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch

/** Show a short message at the bottom of the screen. */
val LocalToast = staticCompositionLocalOf<(String) -> Unit> { {} }

/** Show a message with an action (e.g. Undo): (message, action label,
 *  what the action does). */
val LocalUndo = staticCompositionLocalOf<(String, String, () -> Unit) -> Unit> { { _, _, _ -> } }

private enum class Tab(@StringRes val label: Int, val icon: ImageVector) {
    Devices(R.string.tab_devices, Icons.Filled.Home),
    Notes(R.string.tab_notes, Icons.Filled.Edit),
    Activity(R.string.tab_activity, Icons.Filled.List),
    Settings(R.string.tab_settings, Icons.Filled.Settings),
}

/** Sign-in until ready; then tabs (Devices, Notes, Activity, Settings) plus the
 *  pushed Device, Developer, Automations, Transfers (from Activity), Set up
 *  this phone, Timers and Search (from Home) and Help screens. */
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

    val undo: (String, String, () -> Unit) -> Unit = { msg, label, action ->
        scope.launch {
            snack.currentSnackbarData?.dismiss()
            if (snack.showSnackbar(msg, actionLabel = label, duration = SnackbarDuration.Long) == SnackbarResult.ActionPerformed) action()
        }
    }

    CompositionLocalProvider(LocalToast provides toast, LocalUndo provides undo) {
        if (!Core.ready) {
            Scaffold(snackbarHost = { SnackbarHost(snack) }, containerColor = Color.Transparent) { inner ->
                Box(Modifier.padding(inner)) { SignInScreen(prefs) }
            }
        } else {
            Main(prefs, snack)
            ShareChooser()
        }
    }
}

@Composable
private fun Main(prefs: SharedPreferences, snack: SnackbarHostState) {
    var tab by rememberSaveable { mutableStateOf(Tab.Devices) }
    var device by rememberSaveable { mutableStateOf<String?>(null) }
    var developer by rememberSaveable { mutableStateOf(false) }
    var automations by rememberSaveable { mutableStateOf(false) }
    var transfers by rememberSaveable { mutableStateOf(false) }
    var setup by rememberSaveable { mutableStateOf(false) }
    var timers by rememberSaveable { mutableStateOf(false) }
    // Help: open (the topic list) and the topic shown, if any. Shown over
    // whichever screen opened it, which comes back on Back.
    var help by rememberSaveable { mutableStateOf(false) }
    var helpTopic by rememberSaveable { mutableStateOf<String?>(null) }
    val openHelp = { topic: String? ->
        helpTopic = topic
        help = true
    }

    // Search (from Home): shown over the tabs; what a result opens is pushed
    // over it, so Back comes back to the results as they were. The query,
    // the type chips and the open groups (kind names, comma-separated)
    // survive that and a rotation.
    var search by rememberSaveable { mutableStateOf(false) }
    var searchQuery by rememberSaveable { mutableStateOf("") }
    var searchKinds by rememberSaveable { mutableStateOf("") }
    var searchOpen by rememberSaveable { mutableStateOf("") }
    // What a result opened, for that screen to bring into view and mark.
    var deviceFocus by rememberSaveable { mutableStateOf<String?>(null) }
    var foundTransfer by rememberSaveable { mutableStateOf<String?>(null) }
    var foundAutomation by rememberSaveable { mutableStateOf<String?>(null) }
    var settingsFocus by rememberSaveable { mutableStateOf<String?>(null) }
    var foundNote by rememberSaveable { mutableStateOf<String?>(null) }
    // A result opened a Settings tile or a note (a tab, not a pushed
    // screen): Back goes back to Search. And Help opened from Search closes
    // in one Back.
    var settingsFromSearch by rememberSaveable { mutableStateOf(false) }
    var notesFromSearch by rememberSaveable { mutableStateOf(false) }
    var helpFromSearch by rememberSaveable { mutableStateOf(false) }

    /** Leave Search and everything it opened (another way in took over). */
    val leaveSearch = {
        search = false
        settingsFromSearch = false
        notesFromSearch = false
        helpFromSearch = false
        settingsFocus = null
    }
    val openFound: (Findable) -> Unit = { f ->
        when (f.kind) {
            FoundKind.DEVICES -> { deviceFocus = null; device = f.id }
            FoundKind.MESSAGES -> f.node?.let { deviceFocus = "msg:" + f.id; device = it }
            FoundKind.CLIPBOARD -> f.node?.let { deviceFocus = "clip:" + f.id; device = it }
            FoundKind.TRANSFERS -> { foundTransfer = f.id; transfers = true }
            FoundKind.AUTOMATIONS -> { foundAutomation = f.id; automations = true }
            FoundKind.HELP -> { helpFromSearch = true; openHelp(f.id) }
            FoundKind.NOTES -> {
                foundNote = f.id
                notesFromSearch = true
                search = false
                tab = Tab.Notes
            }
            FoundKind.FOLDERS, FoundKind.AUTOMATION_APPS, FoundKind.SETTINGS -> {
                settingsFocus = when (f.kind) {
                    FoundKind.FOLDERS -> "sync"
                    FoundKind.AUTOMATION_APPS -> "automation-apps"
                    else -> f.id
                }
                settingsFromSearch = true
                search = false
                tab = Tab.Settings
            }
        }
    }
    val closeDevice = { device = null; deviceFocus = null }
    val closeTransfers = { transfers = false; foundTransfer = null }
    val closeAutomations = { automations = false; foundAutomation = null }
    val closeHelp = { help = false; helpFromSearch = false }
    val backToSearch = {
        settingsFromSearch = false
        notesFromSearch = false
        settingsFocus = null
        tab = Tab.Devices
        search = true
    }

    // Link quality on the status lines: refresh while the app is on screen.
    LaunchedEffect(Unit) {
        while (true) {
            kotlinx.coroutines.withContext(kotlinx.coroutines.Dispatchers.IO) { Core.pollQuality() }
            delay(5000)
        }
    }

    // A notification asked for Settings (e.g. to allow location sharing).
    LaunchedEffect(Core.showSettings) {
        if (Core.showSettings) {
            device = null
            developer = false
            automations = false
            transfers = false
            setup = false
            help = false
            leaveSearch()
            tab = Tab.Settings
            Core.showSettings = false
        }
    }

    // A launcher shortcut asked for one device's screen.
    LaunchedEffect(Core.showDevice) {
        Core.showDevice?.let { node ->
            developer = false
            automations = false
            transfers = false
            setup = false
            help = false
            leaveSearch()
            tab = Tab.Devices
            deviceFocus = null
            device = node
            Core.showDevice = null
        }
    }

    // A timer's notification asked for Timers.
    LaunchedEffect(Core.showTimers) {
        if (Core.showTimers) {
            device = null
            developer = false
            automations = false
            transfers = false
            setup = false
            help = false
            leaveSearch()
            tab = Tab.Devices
            timers = true
            Core.showTimers = false
        }
    }

    // A note reminder's notification asked for its note.
    LaunchedEffect(Core.showNote) {
        Core.showNote?.let { id ->
            device = null
            developer = false
            automations = false
            transfers = false
            setup = false
            help = false
            leaveSearch()
            foundNote = id
            tab = Tab.Notes
            Core.showNote = null
        }
    }

    val pushed = help || device != null || developer || automations || transfers || setup || timers || search
    BackHandler(enabled = pushed || settingsFromSearch || notesFromSearch || tab != Tab.Devices) {
        when {
            help && helpTopic != null && !helpFromSearch -> helpTopic = null
            help -> closeHelp()
            device != null -> closeDevice()
            developer -> developer = false
            automations -> closeAutomations()
            transfers -> closeTransfers()
            setup -> setup = false
            timers -> timers = false
            search -> search = false
            settingsFromSearch && tab == Tab.Settings -> backToSearch()
            notesFromSearch && tab == Tab.Notes -> backToSearch()
            else -> tab = Tab.Devices
        }
    }

    Scaffold(
        containerColor = Color.Transparent,
        snackbarHost = { SnackbarHost(snack) },
        bottomBar = {
            if (!pushed) {
                NavigationBar(containerColor = T.bar, tonalElevation = 0.dp) {
                    Tab.entries.forEach { t ->
                        NavigationBarItem(
                            selected = tab == t,
                            onClick = {
                                // Picking a tab leaves what Search opened.
                                settingsFromSearch = false
                                notesFromSearch = false
                                settingsFocus = null
                                tab = t
                            },
                            icon = { Icon(t.icon, contentDescription = null) },
                            label = { Text(stringResource(t.label)) },
                            colors = NavigationBarItemDefaults.colors(
                                // Active navigation is the accent (kit rule).
                                selectedIconColor = T.accentFg, selectedTextColor = T.accentFg,
                                indicatorColor = T.accentBg,
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
                help -> HelpScreen(
                    helpTopic,
                    onTopic = { if (it == null && helpFromSearch) closeHelp() else helpTopic = it },
                    onBack = closeHelp,
                )
                device != null -> DeviceScreen(device!!, onBack = closeDevice, focus = deviceFocus)
                developer -> DeveloperScreen(onBack = { developer = false })
                automations -> AutomationsScreen(onBack = closeAutomations, found = foundAutomation)
                setup -> SetupScreen(onBack = { setup = false }, onHelp = { openHelp("setup-phone") })
                timers -> TimersScreen(onBack = { timers = false }, onHelp = { openHelp("timers") })
                transfers -> TransfersScreen(
                    onBack = closeTransfers,
                    onOpenDevice = { deviceFocus = null; device = it },
                    found = foundTransfer,
                )
                search -> SearchScreen(
                    query = searchQuery,
                    onQuery = { searchQuery = it },
                    kinds = kindsOf(searchKinds),
                    onKinds = { searchKinds = it.joinToString(",") { k -> k.name } },
                    expanded = kindsOf(searchOpen),
                    onExpanded = { searchOpen = it.joinToString(",") { k -> k.name } },
                    onOpen = openFound,
                    onBack = { search = false },
                )
                tab == Tab.Devices -> HomeScreen(
                    onOpen = { deviceFocus = null; device = it },
                    onSetup = { setup = true },
                    onSearch = { search = true },
                    onBattery = { settingsFocus = "battery"; tab = Tab.Settings },
                    onTimers = { timers = true },
                )
                tab == Tab.Notes -> NotesScreen(
                    onHelp = { openHelp("notes") },
                    openNote = foundNote,
                    onOpened = { foundNote = null },
                )
                tab == Tab.Activity -> ActivityScreen(onTransfers = { foundTransfer = null; transfers = true })
                else -> SettingsScreen(
                    prefs,
                    onDeveloper = { developer = true },
                    onAutomations = { foundAutomation = null; automations = true },
                    onSetup = { setup = true },
                    onHelp = { openHelp(null) },
                    focus = settingsFocus,
                )
            }
        }
    }
}

/** Kinds saved as their names, comma-separated (unknown names dropped). */
private fun kindsOf(saved: String): Set<FoundKind> =
    saved.split(',').mapNotNull { name -> FoundKind.entries.firstOrNull { it.name == name } }.toSet()
