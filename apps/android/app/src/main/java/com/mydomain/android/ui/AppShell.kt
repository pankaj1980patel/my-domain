package com.mydomain.android.ui

import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.res.pluralStringResource
import androidx.compose.material3.ExtendedFloatingActionButton
import androidx.compose.animation.core.tween
import androidx.compose.animation.togetherWith
import androidx.compose.animation.slideInHorizontally
import androidx.compose.animation.fadeOut
import androidx.compose.animation.fadeIn
import androidx.compose.animation.ExitTransition
import androidx.compose.animation.EnterTransition
import androidx.compose.animation.AnimatedContent
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.consumeWindowInsets
import androidx.compose.foundation.layout.union
import androidx.compose.foundation.layout.displayCutout
import androidx.compose.foundation.layout.systemBars
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.lazy.LazyListState
import com.mydomain.android.core.ActivityRow
import androidx.compose.material3.LocalContentColor
import android.content.SharedPreferences
import androidx.activity.compose.BackHandler
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.statusBars
import androidx.compose.foundation.layout.windowInsetsPadding
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
import androidx.compose.runtime.mutableStateMapOf
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
import com.mydomain.android.core.Dest
import com.mydomain.android.core.NavStack
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

/** A message with an action (e.g. Undo). `undo(message, label) { … }`
 *  runs the block if the action is tapped; [later] also says what to do
 *  once the message goes without it (the delete itself, held back until
 *  then). */
class UndoHost(private val show: (String, String, () -> Unit, () -> Unit) -> Unit) {
    operator fun invoke(message: String, label: String, onUndo: () -> Unit) = show(message, label, onUndo) {}

    /** [onUndo] if the action is tapped, else [onCommit] once the message
     *  is gone (timed out, swiped, replaced, or the screen went away). */
    fun later(message: String, label: String, onUndo: () -> Unit, onCommit: () -> Unit) = show(message, label, onUndo, onCommit)
}

val LocalUndo = staticCompositionLocalOf { UndoHost { _, _, _, _ -> } }

/** Clears held back while their Undo shows: key ("chat:<node>",
 *  "clips:<node>", "recent-clips") to when it was asked for. Screens hide
 *  what such a clear will remove. */
internal object PendingClears {
    val since = mutableStateMapOf<String, Long>()

    fun pending(key: String): Boolean = since.containsKey(key)

    /** Hidden: something from [at] or before, under a clear waiting on [key]. */
    fun hides(key: String, at: Long): Boolean = since[key]?.let { at <= it } ?: false

    /** Hide now; do [clear] when the Undo goes unused (only if this is
     *  still the clear waiting on [key]). */
    fun ask(undo: UndoHost, key: String, message: String, label: String, clear: () -> Unit) {
        val t0 = System.currentTimeMillis()
        since[key] = t0
        undo.later(message, label, onUndo = { if (since[key] == t0) since.remove(key) }) {
            if (since[key] == t0) {
                clear()
                since.remove(key)
            }
        }
    }
}

private enum class Tab(@StringRes val label: Int, @DrawableRes val icon: Int) {
    Home(R.string.tab_home, R.drawable.ic_lucide_house),
    Devices(R.string.tab_devices, R.drawable.ic_lucide_monitor_smartphone),
    Activity(R.string.tab_activity, R.drawable.ic_lucide_activity),
    More(R.string.tab_more, R.drawable.ic_lucide_layout_grid),
}

/** Sign-in until ready; then the bottom navigation (Home, Devices,
 *  Activity, More) under a top bar (logo, the bell, Settings), plus the
 *  pushed screens ([Dest]: a device, Settings, Notes, Timers, Read later,
 *  Files & transfers, Messages, Automations, Search, the links map, the
 *  health check, Set up this phone, Developer, Help and More's hubs) on a
 *  back stack that Back pops. */
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

    val undo = remember(snack, scope) {
        UndoHost { msg, label, action, commit ->
            scope.launch {
                snack.currentSnackbarData?.dismiss()
                var undone = false
                try {
                    undone = snack.showSnackbar(msg, actionLabel = label, duration = SnackbarDuration.Long) == SnackbarResult.ActionPerformed
                } finally {
                    // Also when this is cancelled (signed out, the app closed):
                    // what was asked for still happens.
                    if (undone) action() else commit()
                }
            }
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
            // A new account's device key, shown once.
            NewAccountKeyDialog()
        }
    }
}

/** The back stack, saved as text (NavStack). */
private val StackSaver = androidx.compose.runtime.saveable.Saver<List<Dest>, ArrayList<String>>(
    save = { NavStack.encode(it) },
    restore = { NavStack.decode(it) },
)

@Composable
private fun Main(prefs: SharedPreferences, snack: SnackbarHostState) {
    var tab by rememberSaveable { mutableStateOf(Tab.Home) }
    // The screens pushed over the tabs, oldest first: Back pops the newest
    // (NavStack), so it goes where you came from.
    var stack by rememberSaveable(stateSaver = StackSaver) { mutableStateOf(emptyList<Dest>()) }
    val push = { d: Dest -> stack = NavStack.push(stack, d) }
    val pop = { stack = NavStack.pop(stack) }
    /** The top screen changed in place (its tab, what it found). */
    val update = { d: Dest -> stack = NavStack.replaceTop(stack, d) }
    /** Only [d] over the tabs (a notification or shortcut took over). */
    val only = { d: Dest -> stack = listOf(d) }
    // Devices: the "Add a device" sheet (its button, and Home's empty state).
    var adding by rememberSaveable { mutableStateOf(false) }
    // Each tab keeps its place; tapping the tab again goes to the top.
    val tabLists = Tab.entries.associateWith { rememberSaveable(saver = LazyListState.Saver) { LazyListState() } }
    val scope = rememberCoroutineScope()

    // Search's query, type chips and open groups (kind names,
    // comma-separated) survive what a result opens and a rotation.
    var searchQuery by rememberSaveable { mutableStateOf("") }
    var searchKinds by rememberSaveable { mutableStateOf("") }
    var searchOpen by rememberSaveable { mutableStateOf("") }

    val openHelp = { topic: String? -> push(Dest(Dest.Kind.Help, topic)) }
    val openDevice = { node: String, focus: String? -> push(Dest(Dest.Kind.Device, node, focus)) }
    val openTransfers = { found: String? -> push(Dest(Dest.Kind.Files, FilesTab.Transfers.key, found)) }
    val openSettings = { focus: String? -> push(Dest(Dest.Kind.Settings, focus)) }
    val openNotes = { found: String? -> push(Dest(Dest.Kind.Notes, found)) }
    val openMessages = { t: MessagesTab ->
        push(Dest(Dest.Kind.Messages, t.name))
        Attention.seenAt = System.currentTimeMillis()
    }
    val openFound: (Findable) -> Unit = { f ->
        when (f.kind) {
            FoundKind.DEVICES -> openDevice(f.id, null)
            FoundKind.MESSAGES -> f.node?.let { openDevice(it, "msg:" + f.id) }
            FoundKind.CLIPBOARD -> f.node?.let { openDevice(it, "clip:" + f.id) }
            FoundKind.TRANSFERS -> openTransfers(f.id)
            FoundKind.AUTOMATIONS -> push(Dest(Dest.Kind.Automations, f.id))
            FoundKind.HELP -> openHelp(f.id)
            FoundKind.READ_LATER -> push(Dest(Dest.Kind.ReadLater, f.id))
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
            a.action == "read_later" -> push(Dest(Dest.Kind.ReadLater, a.target))
            a.action == "timer" -> push(Dest(Dest.Kind.Timers))
            a.node != null -> openDevice(a.node!!, null)
        }
    }

    // The health check: with the server now and every 10 minutes, quickly
    // every minute in between (the badge on More); every 5 and 30 minutes
    // while saving battery, and not at all while the app is off screen
    // (PowerPolicy.healthStep).
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
            if (s != null && !s.finished) push(Dest(Dest.Kind.Onboarding))
        }
    }
    val healthNav = HealthNav(
        onDevice = { openDevice(it, null) },
        onMap = { push(Dest(Dest.Kind.LinksMap)) },
        onSettings = { stack = NavStack.without(stack, Dest.Kind.Health, Dest.Kind.Onboarding) + Dest(Dest.Kind.Settings) },
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
            only(Dest(Dest.Kind.Settings))
            Core.showSettings = false
        }
    }

    // A launcher shortcut asked for one device's screen.
    LaunchedEffect(Core.showDevice) {
        Core.showDevice?.let { node ->
            tab = Tab.Home
            only(Dest(Dest.Kind.Device, node))
            Core.showDevice = null
        }
    }

    // A timer's notification asked for Timers.
    LaunchedEffect(Core.showTimers) {
        if (Core.showTimers) {
            only(Dest(Dest.Kind.Timers))
            Core.showTimers = false
        }
    }

    // A note reminder's notification asked for its note.
    LaunchedEffect(Core.showNote) {
        Core.showNote?.let { id ->
            only(Dest(Dest.Kind.Notes, id))
            Core.showNote = null
        }
    }

    // Debug builds: a screen asked for from adb (md_screen; screenshots).
    // Each request once per shell (by its id), so a shell made after the
    // launch (a new activity's) still gets it, and a rotation doesn't redo it.
    var shownScreen by rememberSaveable { mutableStateOf(0L) }
    LaunchedEffect(Core.showScreen) {
        val want = Core.showScreen ?: return@LaunchedEffect
        if (want.id == shownScreen) return@LaunchedEffect
        shownScreen = want.id
        stack = emptyList()
        adding = false
        tab = Tab.Home
        when (want.screen) {
            DebugScreen.HOME, DebugScreen.SIGN_IN -> Unit
            DebugScreen.DEVICES -> tab = Tab.Devices
            DebugScreen.MORE -> tab = Tab.More
            DebugScreen.NOTES -> openNotes(null)
            DebugScreen.ACTIVITY -> tab = Tab.Activity
            DebugScreen.SETTINGS -> openSettings(want.tab?.let { "group:$it" })
            DebugScreen.DEVICE -> openDevice(want.device ?: "", "tab:" + (want.tab ?: DeviceTab.Overview.key))
            DebugScreen.DEVELOPER -> push(Dest(Dest.Kind.Developer))
            DebugScreen.AUTOMATIONS -> push(Dest(Dest.Kind.Automations))
            DebugScreen.TRANSFERS -> openTransfers(null)
            DebugScreen.FILES -> push(
                Dest(Dest.Kind.Files, (FilesTab.entries.firstOrNull { it.key == want.tab } ?: FilesTab.Files).key),
            )
            DebugScreen.MESSAGES -> openMessages(
                if (want.tab == "notifications") MessagesTab.Notifications else MessagesTab.Chats,
            )
            DebugScreen.SETUP_PHONE -> push(Dest(Dest.Kind.Setup))
            DebugScreen.TIMERS -> push(Dest(Dest.Kind.Timers))
            DebugScreen.READ_LATER -> push(Dest(Dest.Kind.ReadLater))
            DebugScreen.LINKS_MAP -> push(Dest(Dest.Kind.LinksMap))
            DebugScreen.HEALTH -> push(Dest(Dest.Kind.Health))
            DebugScreen.ONBOARDING -> push(Dest(Dest.Kind.Onboarding))
            DebugScreen.HELP -> openHelp(want.topic)
            DebugScreen.SEARCH -> {
                searchQuery = want.query.orEmpty()
                push(Dest(Dest.Kind.Search))
            }
        }
    }

    val top = stack.lastOrNull()
    val pushed = top != null
    // System Back: the newest pushed screen, else back to Home.
    BackHandler(enabled = pushed || tab != Tab.Home) {
        if (pushed) pop() else tab = Tab.Home
    }

    val still = reducedMotion()
    val bad = Health.bad
    val healthState = if (bad > 0) pluralStringResource(R.plurals.health_needs_fixing, bad, bad) else null

    Scaffold(
        containerColor = Color.Transparent,
        snackbarHost = { SnackbarHost(snack) },
        // Status and navigation bars and a landscape cutout, once, here.
        contentWindowInsets = WindowInsets.systemBars.union(WindowInsets.displayCutout),
        topBar = {
            Column {
                if (!pushed) {
                    TopBar(
                        title = if (tab == Tab.Home) null else stringResource(tab.label),
                        bellCount = Attention.newMessages,
                        onBell = { openMessages(if (Attention.newMessages > 0) MessagesTab.Chats else MessagesTab.Notifications) },
                        onSettings = { openSettings(null) },
                    )
                }
                // Offline, the server unreachable, reconnecting: on every
                // screen but the map it opens (StatusBanner.kt).
                if (top?.kind != Dest.Kind.LinksMap) {
                    ConnectionBanner(
                        Modifier.then(if (pushed) Modifier.windowInsetsPadding(WindowInsets.statusBars) else Modifier),
                        onOpen = { push(Dest(Dest.Kind.LinksMap)) },
                    )
                }
            }
        },
        floatingActionButton = {
            // Devices: "Add a device" (here, so a message never covers it).
            if (!pushed && tab == Tab.Devices) {
                ExtendedFloatingActionButton(
                    onClick = { adding = true },
                    containerColor = T.primary,
                    contentColor = T.onPrimary,
                    icon = { LIcon(R.drawable.ic_lucide_plus, T.onPrimary, 22.dp) },
                    text = { Text(stringResource(R.string.join_add_device), fontWeight = FontWeight.Bold) },
                )
            }
        },
        bottomBar = {
            if (!pushed) {
                NavigationBar(containerColor = T.bar, tonalElevation = 0.dp) {
                    Tab.entries.forEach { t ->
                        // Something red in the health check: a dot on More, read out.
                        val dot = t == Tab.More && healthState != null
                        NavigationBarItem(
                            selected = tab == t,
                            onClick = {
                                if (tab == t) scope.launch { tabLists.getValue(t).animateScrollToItem(0) } else tab = t
                            },
                            modifier = if (dot) Modifier.semantics { stateDescription = healthState!! } else Modifier,
                            icon = {
                                if (dot) {
                                    BadgedBox(badge = { Badge(containerColor = T.danger) }) { LIcon(t.icon, LocalContentColor.current, 24.dp) }
                                } else {
                                    LIcon(t.icon, LocalContentColor.current, 24.dp)
                                }
                            },
                            label = { Text(stringResource(t.label), maxLines = 1, overflow = TextOverflow.Ellipsis) },
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
        // The keyboard pushes every screen up, from here only.
        Box(Modifier.fillMaxSize().padding(inner).consumeWindowInsets(inner).imePadding()) {
            AnimatedContent(
                targetState = top to tab,
                // One key per screen: the same screen with another tab or
                // topic doesn't fade (NavStack.key).
                contentKey = { (d, t) -> d?.let(NavStack::key) ?: ("tab:" + t.name) },
                transitionSpec = {
                    if (still) {
                        EnterTransition.None togetherWith ExitTransition.None
                    } else {
                        (fadeIn(tween(180)) + slideInHorizontally(tween(180)) { it / 12 }) togetherWith fadeOut(tween(120))
                    }
                },
                label = "screen",
            ) { (d, t) ->
                if (d != null) {
                    Pushed(
                        d, prefs, healthNav, push, pop, update,
                        search = SearchState(
                            searchQuery, { searchQuery = it },
                            searchKinds, { searchKinds = it }, searchOpen, { searchOpen = it },
                        ),
                        openFound = openFound, openActivity = openActivity,
                    )
                } else {
                    when (t) {
                        Tab.Home -> HomeScreen(
                            onOpen = { openDevice(it, null) },
                            onOpenFocus = { n, f -> openDevice(n, f) },
                            onRename = { openDevice(it, "edit") },
                            onDevices = { tab = Tab.Devices },
                            onAddDevice = { tab = Tab.Devices; adding = true },
                            onBattery = { openSettings("battery") },
                            onActivity = { tab = Tab.Activity },
                            onActivityRow = openActivity,
                            onOnboarding = { push(Dest(Dest.Kind.Onboarding)) },
                            state = tabLists.getValue(Tab.Home),
                        )
                        Tab.Devices -> DevicesScreen(
                            onOpen = { openDevice(it, null) },
                            state = tabLists.getValue(Tab.Devices),
                        )
                        Tab.Activity -> ActivityScreen(
                            onTransfers = { openTransfers(null) },
                            onOpen = openActivity,
                            state = tabLists.getValue(Tab.Activity),
                        )
                        Tab.More -> MoreScreen(state = tabLists.getValue(Tab.More)) { item ->
                            when (item) {
                                MoreItem.Files -> push(Dest(Dest.Kind.Files, FilesTab.Files.key))
                                MoreItem.Messages -> openMessages(MessagesTab.Chats)
                                MoreItem.Workspace -> push(Dest(Dest.Kind.Workspace))
                                MoreItem.Automations -> push(Dest(Dest.Kind.Automations))
                                MoreItem.Search -> push(Dest(Dest.Kind.Search))
                                MoreItem.ConnHealth -> push(Dest(Dest.Kind.ConnHealth))
                            }
                        }
                    }
                }
            }
        }
    }
    // "Add a device": the join invite in a sheet (Back closes it).
    if (adding && !pushed) AddDeviceSheet(onClose = { adding = false })
}

/** Search's saved state, handed to its screen. */
private class SearchState(
    val query: String,
    val onQuery: (String) -> Unit,
    val kinds: String,
    val onKinds: (String) -> Unit,
    val open: String,
    val onOpen: (String) -> Unit,
)

/** The screen [d] names, with where its buttons go. */
@Composable
private fun Pushed(
    d: Dest,
    prefs: SharedPreferences,
    healthNav: HealthNav,
    push: (Dest) -> Unit,
    pop: () -> Unit,
    update: (Dest) -> Unit,
    search: SearchState,
    openFound: (Findable) -> Unit,
    openActivity: (ActivityRow) -> Unit,
) {
    val help = { topic: String -> push(Dest(Dest.Kind.Help, topic)) }
    val openDevice = { n: String, f: String? -> push(Dest(Dest.Kind.Device, n, f)) }
    when (d.kind) {
        Dest.Kind.Help -> HelpScreen(
            d.a,
            // A topic (or a link in one) is pushed; its own back arrow pops.
            onTopic = { if (it == null) pop() else push(Dest(Dest.Kind.Help, it)) },
            onBack = pop,
        )
        Dest.Kind.Device -> DeviceScreen(
            d.a ?: "", onBack = pop, focus = d.b,
            onHealth = { push(Dest(Dest.Kind.Health)) },
        )
        Dest.Kind.Developer -> DeveloperScreen(onBack = pop)
        Dest.Kind.Automations -> AutomationsScreen(onBack = pop, found = d.a)
        Dest.Kind.Setup -> SetupScreen(onBack = pop, onHelp = { help("setup-phone") })
        Dest.Kind.Timers -> TimersScreen(onBack = pop, onHelp = { help("timers") })
        Dest.Kind.ReadLater -> ReadLaterScreen(onBack = pop, onHelp = { help("read-later") }, found = d.a)
        Dest.Kind.LinksMap -> LinksMapScreen(
            onBack = pop,
            onHelp = { help("troubleshooting") },
            onDevice = { openDevice(it, null) },
        )
        Dest.Kind.Onboarding -> OnboardingScreen(onClose = pop, nav = healthNav)
        Dest.Kind.Health -> HealthScreen(onBack = pop, nav = healthNav)
        Dest.Kind.Notes -> NotesScreen(
            onHelp = { help("notes") },
            openNote = d.a,
            onOpened = { update(d.copy(a = null)) },
            onBack = pop,
        )
        Dest.Kind.Settings -> SettingsScreen(
            prefs,
            onDeveloper = { push(Dest(Dest.Kind.Developer)) },
            onAutomations = { push(Dest(Dest.Kind.Automations)) },
            onSetup = { push(Dest(Dest.Kind.Setup)) },
            onHelp = { push(Dest(Dest.Kind.Help)) },
            onOnboarding = { push(Dest(Dest.Kind.Onboarding)) },
            focus = d.a,
            onBack = pop,
        )
        Dest.Kind.Files -> FilesHubScreen(
            FilesTab.entries.firstOrNull { it.key == d.a } ?: FilesTab.Files,
            { update(Dest(Dest.Kind.Files, it.key, d.b)) }, onBack = pop,
            onOpenDevice = { n, f -> openDevice(n, f) },
            foundTransfer = d.b,
        )
        Dest.Kind.Messages -> MessagesHubScreen(
            MessagesTab.entries.firstOrNull { it.name == d.a } ?: MessagesTab.Chats,
            { update(Dest(Dest.Kind.Messages, it.name)) }, onBack = pop,
            onOpenDevice = { n, f -> openDevice(n, f) },
            onActivityRow = openActivity,
        )
        Dest.Kind.Search -> SearchScreen(
            query = search.query,
            onQuery = search.onQuery,
            kinds = kindsOf(search.kinds),
            onKinds = { search.onKinds(it.joinToString(",") { k -> k.name }) },
            expanded = kindsOf(search.open),
            onExpanded = { search.onOpen(it.joinToString(",") { k -> k.name }) },
            onOpen = openFound,
            onBack = pop,
        )
        Dest.Kind.Workspace -> WorkspaceScreen(onBack = pop) { item ->
            when (item) {
                WorkItem.Notes -> push(Dest(Dest.Kind.Notes))
                WorkItem.Timers -> push(Dest(Dest.Kind.Timers))
                WorkItem.ReadLater -> push(Dest(Dest.Kind.ReadLater))
            }
        }
        Dest.Kind.ConnHealth -> ConnHealthScreen(
            onBack = pop,
            onMap = { push(Dest(Dest.Kind.LinksMap)) },
            onHealth = { push(Dest(Dest.Kind.Health)) },
        )
    }
}

/** Kinds saved as their names, comma-separated (unknown names dropped). */
private fun kindsOf(saved: String): Set<FoundKind> =
    saved.split(',').mapNotNull { name -> FoundKind.entries.firstOrNull { it.name == name } }.toSet()
