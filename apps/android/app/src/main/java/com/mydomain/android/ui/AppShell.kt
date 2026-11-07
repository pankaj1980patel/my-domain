package com.mydomain.android.ui

import com.mydomain.android.core.ActivityRow
import androidx.compose.material3.LocalContentColor
import androidx.compose.runtime.mutableLongStateOf
import android.content.SharedPreferences
import androidx.activity.compose.BackHandler
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.annotation.StringRes
import androidx.annotation.DrawableRes
import androidx.compose.foundation.layout.padding
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
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import com.mydomain.android.R
import com.mydomain.android.core.Account
import com.mydomain.android.core.Core
import com.mydomain.android.core.DebugScreen
import com.mydomain.android.core.Findable
import com.mydomain.android.core.FoundKind
import com.mydomain.android.core.Health
import com.mydomain.android.core.PowerPolicy
import com.mydomain.android.PowerSaver
import androidx.compose.material3.Badge
import androidx.compose.material3.BadgedBox
import androidx.compose.ui.platform.LocalContext
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch

/** Show a short message at the bottom of the screen. */
val LocalToast = staticCompositionLocalOf<(String) -> Unit> { {} }

/** Show a message with an action (e.g. Undo): (message, action label,
 *  what the action does). */
val LocalUndo = staticCompositionLocalOf<(String, String, () -> Unit) -> Unit> { { _, _, _ -> } }

private enum class Tab(@StringRes val label: Int, @DrawableRes val icon: Int) {
    Home(R.string.tab_home, R.drawable.ic_lucide_house),
    Devices(R.string.tab_devices, R.drawable.ic_lucide_monitor_smartphone),
    Activity(R.string.tab_activity, R.drawable.ic_lucide_activity),
    More(R.string.tab_more, R.drawable.ic_lucide_layout_grid),
}

/** Sign-in until ready; then the bottom navigation (Home, Devices,
 *  Activity, More) under a top bar (logo, the bell, Settings), plus the
 *  pushed screens: a device, Settings, Notes, Timers, Read later, Files &
 *  transfers, Messages, Workspace, Automations, Search, the links map, the
 *  health check, Set up this phone, Developer and Help. */
@Composable
fun App(prefs: SharedPreferences) {
    val snack = remember { SnackbarHostState() }
    val scope = rememberCoroutineScope()
    val toast: (String) -> Unit = { msg -> scope.launch { snack.currentSnackbarData?.dismiss(); snack.showSnackbar(msg) } }

    // A background sign-in (device lab) can finish while sign-in shows.
    LaunchedEffect(Core.ready) {
        while (!Core.ready) {
            Core.awaitOnScreen()
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
            // Signed out by the server (password changed elsewhere...): the
            // password alone signs back in; else the full sign-in.
            val user = Core.session?.username ?: prefs.getString("user", null)
            Scaffold(snackbarHost = { SnackbarHost(snack) }, containerColor = Color.Transparent) { inner ->
                Box(Modifier.padding(inner)) {
                    if (Account.signedOut && !user.isNullOrBlank()) SignedOutScreen(user) else SignInScreen(prefs)
                }
            }
        } else {
            Main(prefs, snack)
            ShareChooser()
            // "Download on…" for a shared link, and another device asking
            // this phone to download one.
            DownloadOnChooser()
            DownloadAskDialog()
            // A verify code scanned with the camera app: what came of it.
            VerifyScanResult()
        }
    }
}

/** What the top bar's bell counts: messages that came in since you last
 *  opened Messages (this session only). */
private object Attention {
    var seenAt by mutableLongStateOf(0L)

    val newMessages: Int
        get() = Core.messages.count { it.incoming && it.ts > seenAt }
}

@Composable
private fun Main(prefs: SharedPreferences, snack: SnackbarHostState) {
    var tab by rememberSaveable { mutableStateOf(Tab.Home) }
    var device by rememberSaveable { mutableStateOf<String?>(null) }
    var developer by rememberSaveable { mutableStateOf(false) }
    var automations by rememberSaveable { mutableStateOf(false) }
    var setup by rememberSaveable { mutableStateOf(false) }
    var timers by rememberSaveable { mutableStateOf(false) }
    var readLater by rememberSaveable { mutableStateOf(false) }
    var linksMap by rememberSaveable { mutableStateOf(false) }
    // Notes and Settings (tabs before the redesign), and More's hubs.
    var notes by rememberSaveable { mutableStateOf(false) }
    var settings by rememberSaveable { mutableStateOf(false) }
    var filesHub by rememberSaveable { mutableStateOf(false) }
    var filesTab by rememberSaveable { mutableStateOf(FilesTab.Files) }
    var messages by rememberSaveable { mutableStateOf(false) }
    var messagesTab by rememberSaveable { mutableStateOf(MessagesTab.Chats) }
    var workspace by rememberSaveable { mutableStateOf(false) }
    // The health check and first-run setup (HealthScreen.kt).
    var health by rememberSaveable { mutableStateOf(false) }
    var onboarding by rememberSaveable { mutableStateOf(false) }
    // Help: open (the topic list) and the topic shown, if any. Shown over
    // whichever screen opened it, which comes back on Back.
    var help by rememberSaveable { mutableStateOf(false) }
    var helpTopic by rememberSaveable { mutableStateOf<String?>(null) }
    val openHelp = { topic: String? ->
        helpTopic = topic
        help = true
    }

    // Search (from More): shown over the tabs; what a result opens is pushed
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
    var foundLink by rememberSaveable { mutableStateOf<String?>(null) }
    // Help opened from Search closes in one Back.
    var helpFromSearch by rememberSaveable { mutableStateOf(false) }

    val openDevice = { node: String, focus: String? -> deviceFocus = focus; device = node }
    val openTransfers = { found: String? ->
        foundTransfer = found
        filesTab = FilesTab.Transfers
        filesHub = true
    }
    val openSettings = { focus: String? -> settingsFocus = focus; settings = true }
    val openNotes = { found: String? -> foundNote = found; notes = true }
    val openFound: (Findable) -> Unit = { f ->
        when (f.kind) {
            FoundKind.DEVICES -> openDevice(f.id, null)
            FoundKind.MESSAGES -> f.node?.let { openDevice(it, "msg:" + f.id) }
            FoundKind.CLIPBOARD -> f.node?.let { openDevice(it, "clip:" + f.id) }
            FoundKind.TRANSFERS -> openTransfers(f.id)
            FoundKind.AUTOMATIONS -> { foundAutomation = f.id; automations = true }
            FoundKind.HELP -> { helpFromSearch = true; openHelp(f.id) }
            FoundKind.READ_LATER -> { foundLink = f.id; readLater = true }
            FoundKind.NOTES -> openNotes(f.id)
            FoundKind.FOLDERS, FoundKind.AUTOMATION_APPS, FoundKind.SETTINGS -> openSettings(
                when (f.kind) {
                    FoundKind.FOLDERS -> "sync"
                    FoundKind.AUTOMATION_APPS -> "automation-apps"
                    else -> f.id
                },
            )
        }
    }
    // An Activity row opens where it lives: the found-item ways Search uses.
    val openActivity: (ActivityRow) -> Unit = { a ->
        when {
            a.kind == "files" -> openTransfers(a.target)
            a.kind == "messages" && a.node != null -> openDevice(a.node!!, "msg:" + a.target)
            a.kind == "clipboard" && a.node != null -> openDevice(a.node!!, "clip:" + a.target)
            a.action == "note" -> openNotes(a.target)
            a.action == "read_later" -> { foundLink = a.target; readLater = true }
            a.action == "timer" -> timers = true
            a.node != null -> openDevice(a.node!!, null)
        }
    }
    val closeDevice = { device = null; deviceFocus = null }
    val closeFiles = { filesHub = false; foundTransfer = null }
    val closeAutomations = { automations = false; foundAutomation = null }
    val closeHelp = { help = false; helpFromSearch = false }
    val closeReadLater = { readLater = false; foundLink = null }
    val closeSettings = { settings = false; settingsFocus = null }
    val closeNotes = { notes = false; foundNote = null }
    val openMessages = { t: MessagesTab ->
        messagesTab = t
        messages = true
        Attention.seenAt = System.currentTimeMillis()
    }
    /** Close every pushed screen (a notification or shortcut took over). */
    val closeAll = {
        closeDevice()
        developer = false
        closeAutomations()
        setup = false
        timers = false
        closeReadLater()
        linksMap = false
        health = false
        onboarding = false
        closeHelp()
        helpTopic = null
        closeNotes()
        closeSettings()
        closeFiles()
        messages = false
        workspace = false
        search = false
    }

    // The health check: with the server now and every 10 minutes, quickly
    // every minute in between (the Home card and the Settings badge);
    // every 5 and 30 minutes while saving battery, and not at all while the
    // app is off screen (PowerPolicy.healthStep).
    val ctx = LocalContext.current
    LaunchedEffect(Unit) {
        var lastRun: Long? = null
        var lastServer: Long? = null
        while (true) {
            Core.awaitOnScreen()
            val now = System.currentTimeMillis()
            val step = PowerPolicy.healthStep(now, lastRun, lastServer, PowerSaver.current.saving)
            if (step.run) {
                Health.run(ctx, server = step.server)
                lastRun = now
                if (step.server) lastServer = now
            }
            delay(step.waitMs)
        }
    }
    // Signed in or joined just now: first-run setup, unless done here before.
    LaunchedEffect(Health.justSignedIn) {
        if (Health.justSignedIn) {
            Health.justSignedIn = false
            val s = Health.loadSetup()
            if (s != null && !s.finished) onboarding = true
        }
    }
    val healthNav = HealthNav(
        onDevice = { openDevice(it, null) },
        onMap = { linksMap = true },
        onSettings = { health = false; onboarding = false; openSettings(null) },
        onHelp = { openHelp(it) },
    )

    // Link quality on the status lines: refresh while the app is on screen.
    LaunchedEffect(Unit) {
        while (true) {
            Core.awaitOnScreen()
            kotlinx.coroutines.withContext(kotlinx.coroutines.Dispatchers.IO) { Core.pollQuality() }
            delay(5000)
        }
    }

    // A notification asked for Settings (e.g. to allow location sharing).
    LaunchedEffect(Core.showSettings) {
        if (Core.showSettings) {
            closeAll()
            openSettings(null)
            Core.showSettings = false
        }
    }

    // A launcher shortcut asked for one device's screen.
    LaunchedEffect(Core.showDevice) {
        Core.showDevice?.let { node ->
            closeAll()
            tab = Tab.Home
            openDevice(node, null)
            Core.showDevice = null
        }
    }

    // A timer's notification asked for Timers.
    LaunchedEffect(Core.showTimers) {
        if (Core.showTimers) {
            closeAll()
            timers = true
            Core.showTimers = false
        }
    }

    // A note reminder's notification asked for its note.
    LaunchedEffect(Core.showNote) {
        Core.showNote?.let { id ->
            closeAll()
            openNotes(id)
            Core.showNote = null
        }
    }

    // Debug builds: a screen asked for from adb (md_screen; screenshots).
    LaunchedEffect(Core.showScreen) {
        val want = Core.showScreen ?: return@LaunchedEffect
        Core.showScreen = null
        closeAll()
        tab = Tab.Home
        when (want.screen) {
            DebugScreen.HOME, DebugScreen.SIGN_IN -> Unit
            DebugScreen.DEVICES -> tab = Tab.Devices
            DebugScreen.MORE -> tab = Tab.More
            DebugScreen.NOTES -> openNotes(null)
            DebugScreen.ACTIVITY -> tab = Tab.Activity
            DebugScreen.SETTINGS -> openSettings(null)
            DebugScreen.DEVICE -> openDevice(want.device ?: "", null)
            DebugScreen.DEVELOPER -> developer = true
            DebugScreen.AUTOMATIONS -> automations = true
            DebugScreen.TRANSFERS -> openTransfers(null)
            DebugScreen.FILES -> { filesTab = FilesTab.Files; filesHub = true }
            DebugScreen.MESSAGES -> openMessages(MessagesTab.Chats)
            DebugScreen.WORKSPACE -> workspace = true
            DebugScreen.SETUP_PHONE -> setup = true
            DebugScreen.TIMERS -> timers = true
            DebugScreen.READ_LATER -> readLater = true
            DebugScreen.LINKS_MAP -> linksMap = true
            DebugScreen.HEALTH -> health = true
            DebugScreen.ONBOARDING -> onboarding = true
            DebugScreen.HELP -> openHelp(want.topic)
            DebugScreen.SEARCH -> {
                searchQuery = want.query.orEmpty()
                search = true
            }
        }
    }

    val pushed = help || device != null || developer || automations || setup || timers || readLater || linksMap ||
        health || onboarding || notes || settings || filesHub || messages || workspace || search
    BackHandler(enabled = pushed || tab != Tab.Home) {
        when {
            help && helpTopic != null && !helpFromSearch -> helpTopic = null
            help -> closeHelp()
            device != null -> closeDevice()
            developer -> developer = false
            automations -> closeAutomations()
            setup -> setup = false
            timers -> timers = false
            readLater -> closeReadLater()
            linksMap -> linksMap = false
            onboarding -> onboarding = false
            health -> health = false
            notes -> closeNotes()
            settings -> closeSettings()
            filesHub -> closeFiles()
            messages -> messages = false
            workspace -> workspace = false
            search -> search = false
            else -> tab = Tab.Home
        }
    }

    Scaffold(
        containerColor = Color.Transparent,
        snackbarHost = { SnackbarHost(snack) },
        topBar = {
            if (!pushed) {
                TopBar(
                    bellCount = Attention.newMessages,
                    onBell = { openMessages(if (Attention.newMessages > 0) MessagesTab.Chats else MessagesTab.Notifications) },
                    settingsDot = Health.bad > 0,
                    onSettings = { openSettings(null) },
                )
            }
        },
        bottomBar = {
            if (!pushed) {
                NavigationBar(containerColor = T.bar, tonalElevation = 0.dp) {
                    Tab.entries.forEach { t ->
                        NavigationBarItem(
                            selected = tab == t,
                            onClick = { tab = t },
                            icon = {
                                // Something red in the health check: a dot on More.
                                if (t == Tab.More && Health.bad > 0) {
                                    BadgedBox(badge = { Badge(containerColor = T.danger) }) { LIcon(t.icon, LocalContentColor.current, 24.dp) }
                                } else {
                                    LIcon(t.icon, LocalContentColor.current, 24.dp)
                                }
                            },
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
                device != null -> DeviceScreen(device!!, onBack = closeDevice, focus = deviceFocus, onHealth = { closeDevice(); health = true })
                developer -> DeveloperScreen(onBack = { developer = false })
                automations -> AutomationsScreen(onBack = closeAutomations, found = foundAutomation)
                setup -> SetupScreen(onBack = { setup = false }, onHelp = { openHelp("setup-phone") })
                timers -> TimersScreen(onBack = { timers = false }, onHelp = { openHelp("timers") })
                readLater -> ReadLaterScreen(onBack = closeReadLater, onHelp = { openHelp("read-later") }, found = foundLink)
                linksMap -> LinksMapScreen(
                    onBack = { linksMap = false },
                    onHelp = { openHelp("troubleshooting") },
                    onDevice = { openDevice(it, null) },
                )
                onboarding -> OnboardingScreen(onClose = { onboarding = false }, nav = healthNav)
                health -> HealthScreen(onBack = { health = false }, nav = healthNav)
                notes -> NotesScreen(
                    onHelp = { openHelp("notes") },
                    openNote = foundNote,
                    onOpened = { foundNote = null },
                    onBack = closeNotes,
                )
                settings -> SettingsScreen(
                    prefs,
                    onDeveloper = { developer = true },
                    onAutomations = { foundAutomation = null; automations = true },
                    onSetup = { setup = true },
                    onHelp = { openHelp(null) },
                    onHealth = { health = true },
                    onOnboarding = { onboarding = true },
                    focus = settingsFocus,
                    onBack = closeSettings,
                )
                filesHub -> FilesHubScreen(
                    filesTab, { filesTab = it }, onBack = closeFiles,
                    onOpenDevice = { n, f -> openDevice(n, f) },
                    foundTransfer = foundTransfer,
                )
                messages -> MessagesHubScreen(
                    messagesTab, { messagesTab = it }, onBack = { messages = false },
                    onOpenDevice = { n, f -> openDevice(n, f) },
                    onActivityRow = openActivity,
                )
                workspace -> WorkspaceScreen(
                    onBack = { workspace = false },
                    onNotes = { openNotes(null) },
                    onTimers = { timers = true },
                    onReadLater = { foundLink = null; readLater = true },
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
                tab == Tab.Home -> HomeScreen(
                    onOpen = { openDevice(it, null) },
                    onOpenTab = { n, t -> openDevice(n, "tab:$t") },
                    onRename = { openDevice(it, "edit") },
                    onDevices = { tab = Tab.Devices },
                    onBattery = { openSettings("battery") },
                    onActivity = { tab = Tab.Activity },
                    onActivityRow = openActivity,
                    onHealth = { health = true },
                    onOnboarding = { onboarding = true },
                )
                tab == Tab.Devices -> DevicesScreen(onOpen = { openDevice(it, null) })
                tab == Tab.Activity -> ActivityScreen(
                    onTransfers = { openTransfers(null) },
                    onOpen = openActivity,
                )
                else -> MoreScreen { item ->
                    when (item) {
                        MoreItem.Files -> { filesTab = FilesTab.Files; filesHub = true }
                        MoreItem.Messages -> openMessages(MessagesTab.Chats)
                        MoreItem.Workspace -> workspace = true
                        MoreItem.Notes -> openNotes(null)
                        MoreItem.Timers -> timers = true
                        MoreItem.ReadLater -> { foundLink = null; readLater = true }
                        MoreItem.Automations -> { foundAutomation = null; automations = true }
                        MoreItem.Search -> search = true
                        MoreItem.Connections -> linksMap = true
                        MoreItem.Health -> health = true
                        MoreItem.Settings -> openSettings(null)
                        MoreItem.Help -> openHelp(null)
                    }
                }
            }
        }
    }
}

/** Kinds saved as their names, comma-separated (unknown names dropped). */
private fun kindsOf(saved: String): Set<FoundKind> =
    saved.split(',').mapNotNull { name -> FoundKind.entries.firstOrNull { it.name == name } }.toSet()
