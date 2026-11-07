package com.mydomain.android.core

import com.mydomain.android.ffi.NoteItem
import com.mydomain.android.ffi.ReadLaterEntry
import com.mydomain.android.ffi.TimerItem

/**
 * Made-up devices and what they said, for screenshots in debug builds
 * (`--ez md_sample true`, [DebugScreens]): never the user's data. Core
 * shows these instead of the engine's while sample mode is on
 * ([Core.startSample]); they live in memory only and nothing here is saved.
 * Pure: tested on the JVM.
 */
object SampleData {
    /** Every sample node id starts with this (so none can be a real one). */
    const val PREFIX = "sample-"

    const val ME = "sample-pixel8"
    const val MACBOOK = "sample-macbook"
    const val MINI = "sample-mac-mini"
    const val OFFICE = "sample-office-pc"
    const val LAB = "sample-lab-laptop"

    /** One of your devices: [link] its live link's transport (null: none),
     *  [stage] the connect ladder's stage when there's no link. */
    data class Device(
        val node: String,
        val name: String,
        val ip: String,
        val source: String,
        val link: String?,
        val quality: String?,
        val stage: String?,
        val kind: DeviceKind,
    )

    const val MY_NAME = "Pixel 8"
    const val USER = "pankaj"
    const val SERVER = "https://sync.example.com"
    const val MY_IP = "192.168.1.31"

    val devices = listOf(
        Device(MACBOOK, "Pankaj’s MacBook Pro", "192.168.1.24", "lan", "quic", "excellent", null, DeviceKind("macos", "laptop", "MacBook Pro", "Apple", "mac_laptop")),
        Device(MINI, "Home Mac mini", "192.168.1.10", "lan", "quic", "good", null, DeviceKind("macos", "desktop", "Mac mini", "Apple", "mac_desktop")),
        Device(LAB, "Lab laptop", "100.71.20.5", "registry", "relay", "good", null, DeviceKind("linux", "laptop", "ThinkPad X1 Carbon", "Lenovo", "linux_laptop")),
        Device(OFFICE, "Office PC", "10.20.0.41", "registry", null, null, "asleep", DeviceKind("windows", "desktop", "OptiPlex 7010", "Dell", "win_desktop")),
    )

    /** Wallpaper colours some devices shared (the others show the
     *  built-in art). */
    val wallpapers = mapOf(
        MACBOOK to listOf("#1b3a8a", "#6d4bd8", "#f08a5d"),
        LAB to listOf("#0f3d3e", "#1fa29a", "#a7e8bd"),
    )

    private const val GB = 1024L * 1024 * 1024
    private const val MB = 1024L * 1024
    private const val MIN = 60_000L
    private const val HOUR = 60 * MIN

    /** What each connected computer said about itself. */
    fun statuses(now: Long): Map<String, DeviceStatus> {
        val computer = setOf("volume", "mute", "media", "lock", "search", "slides", "input", "screen", "keep_awake")
        return mapOf(
            MACBOOK to DeviceStatus(
                battery = 54, charging = true, actions = computer + setOf("locate", "run"),
                volume = 30, muted = false, ramFree = 6 * GB, ramTotal = 16 * GB,
                storageFree = 212 * GB, storageTotal = 494 * GB, cpu = 23, uptimeS = 3 * 86_400L + 4 * 3_600L,
                sharedFolders = 1, commands = listOf(RemoteCommand("c1", "Back up photos"), RemoteCommand("c2", "Empty Downloads")),
                screenView = true, audioSink = true, audioSource = true, platform = "desktop", mediaCtl = true, seenAt = now,
            ),
            MINI to DeviceStatus(
                battery = null, charging = null, actions = computer,
                volume = 45, muted = false, ramFree = 9 * GB, ramTotal = 16 * GB,
                storageFree = 340 * GB, storageTotal = 512 * GB, cpu = 12, uptimeS = 9 * 86_400L,
                screenView = true, audioSink = true, platform = "desktop", mediaCtl = true, seenAt = now,
            ),
            LAB to DeviceStatus(
                battery = 91, charging = false, actions = computer - "slides",
                volume = 55, muted = false, ramFree = 9 * GB, ramTotal = 32 * GB,
                storageFree = 88 * GB, storageTotal = 256 * GB, cpu = 61, uptimeS = 11 * 3_600L,
                screenView = true, platform = "desktop", seenAt = now,
            ),
        )
    }

    /** What plays on the computers. */
    fun playing(now: Long): Map<String, Playing> = mapOf(
        MACBOOK to Playing(
            title = "Clair de Lune", artist = "Claude Debussy", app = "Music", playing = true,
            duration = 312_000, position = 104_000, at = now, canSeek = true, volume = 6, volumeMax = 15,
        ),
    )

    fun messages(now: Long): List<ChatMessage> = listOf(
        ChatMessage(1, MACBOOK, incoming = false, text = "Can you send me the boarding pass?", ts = now - 3 * HOUR, delivered = true),
        ChatMessage(2, MACBOOK, incoming = true, text = "Sent it to your phone just now", ts = now - 3 * HOUR + 2 * MIN),
        ChatMessage(3, MACBOOK, incoming = false, text = "Got it, thanks!", ts = now - 3 * HOUR + 3 * MIN, delivered = true),
        ChatMessage(4, MINI, incoming = true, text = "The video export finished", ts = now - 40 * MIN),
    )

    fun transfers(): List<Transfer> = listOf(
        Transfer("t1", MACBOOK, "Holiday video.mov", incoming = true, done = 860 * MB, total = 1_200 * MB),
        Transfer("t2", MACBOOK, "Boarding pass.pdf", incoming = true, done = 380 * 1024, total = 380 * 1024, state = TransferState.Done, mime = "application/pdf"),
        Transfer("t3", MINI, "IMG_20261104_181522.jpg", incoming = false, done = 4 * MB, total = 4 * MB, state = TransferState.Done, mime = "image/jpeg"),
        Transfer("t4", OFFICE, "Invoice-October.xlsx", incoming = false, done = 600 * 1024, total = 2 * MB, state = TransferState.Failed, reason = "offline"),
    )

    /** The Transfers screen's list ([TransferLog.list] in sample mode). */
    fun transferRecords(now: Long = System.currentTimeMillis()): List<TransferRecord> = listOf(
        TransferRecord(
            "t1", MACBOOK, "Holiday video.mov", incoming = true, done = 860 * MB, total = 1_200 * MB,
            started = now - 30_000, rate = 38.0 * MB, movedAt = now,
        ),
        TransferRecord(
            "t2", MACBOOK, "Boarding pass.pdf", incoming = true, state = TransferState.Done, done = 380 * 1024, total = 380 * 1024,
            mime = "application/pdf", started = now - 3 * HOUR, ended = now - 3 * HOUR,
        ),
        TransferRecord(
            "t3", MINI, "IMG_20261104_181522.jpg", incoming = false, state = TransferState.Done, done = 4 * MB, total = 4 * MB,
            mime = "image/jpeg", started = now - 4 * HOUR, ended = now - 4 * HOUR,
        ),
        TransferRecord(
            "t4", OFFICE, "Invoice-October.xlsx", incoming = false, state = TransferState.Failed, done = 600 * 1024, total = 2 * MB,
            reason = "offline", started = now - 5 * HOUR, ended = now - 5 * HOUR,
        ),
    )

    /** The Activity timeline's own record ([ActivityLog] in sample mode). */
    fun activityLog(now: Long = System.currentTimeMillis()): List<ActivityEntry> = listOf(
        ActivityEntry(1, "link_up", MACBOOK, now - 20 * MIN),
        ActivityEntry(2, "ring", LAB, now - 2 * HOUR),
        ActivityEntry(3, "open_url", MINI, now - 3 * HOUR, subject = "https://example.com/talks/local-first-software"),
        ActivityEntry(4, "link_down", OFFICE, now - 6 * HOUR),
    )

    fun clips(now: Long): List<ClipEntry> = listOf(
        ClipEntry("Meet at the north gate at 6:30", null, now - 6 * MIN),
        ClipEntry("https://example.com/talks/local-first-software", MACBOOK, now - 2 * HOUR),
        ClipEntry("Order #48213-AX", MINI, now - 3 * HOUR),
    )

    fun activity(now: Long): List<ActivityItem> = listOf(
        ActivityItem(1, now - 2 * MIN, "Receiving Holiday video.mov from Pankaj’s MacBook Pro", Tone.Info),
        ActivityItem(2, now - 40 * MIN, "Home Mac mini: The video export finished", Tone.Info),
        ActivityItem(3, now - 3 * HOUR, "Saved Boarding pass.pdf from Pankaj’s MacBook Pro", Tone.Ok),
        ActivityItem(4, now - 5 * HOUR, "Couldn’t send Invoice-October.xlsx to Office PC", Tone.Error),
    )

    /** The health check's report: one device worth a look. */
    fun healthChecks(): List<HealthPlan.Check> = listOf(
        HealthPlan.Check("link", "warn", "asleep", nodeId = OFFICE, name = "Office PC", fix = "connect"),
        HealthPlan.Check("server", "ok", "ok"),
        HealthPlan.Check("push", "ok", "working"),
        HealthPlan.Check("disk", "ok", "ok", detail = "48213"),
        HealthPlan.Check("link", "ok", "direct", nodeId = MACBOOK, name = "Pankaj’s MacBook Pro", detail = "lan"),
        HealthPlan.Check("link", "ok", "direct", nodeId = MINI, name = "Home Mac mini", detail = "lan"),
        HealthPlan.Check("link", "info", "relay", nodeId = LAB, name = "Lab laptop"),
    )

    /** How the sample devices are linked (the links map). */
    fun linksMap(): LinksMapInfo = LinksMapInfo(
        nodeId = ME,
        name = MY_NAME,
        devices = listOf(
            MapDeviceInfo(MINI, "Home Mac mini", MapLinkInfo("lan", "quic", "192.168.1.10:47103", "local", 4, 0.0, 5400, 1), sharesLinks = true, verified = true),
            MapDeviceInfo(LAB, "Lab laptop", MapLinkInfo("relay", "quic", null, null, 86, 0.4, 1800, 2), sharesLinks = true),
            MapDeviceInfo(OFFICE, "Office PC", null, reason = "asleep", lastSeenAgoS = 6 * 3600),
            MapDeviceInfo(MACBOOK, "Pankaj’s MacBook Pro", MapLinkInfo("lan", "quic", "192.168.1.24:47103", "local", 3, 0.0, 1200, 1), sharesLinks = true, verified = true),
        ),
        others = listOf(
            OtherLinkInfo(MACBOOK, MINI, "lan", 2, MACBOOK, 30),
            OtherLinkInfo(MINI, LAB, "relay", 74, MINI, 45),
        ),
    )

    fun notes(now: Long): List<NoteItem> = listOf(
        NoteItem("n1", "Groceries", "[ ] Oat milk\n[x] Coffee beans\n[ ] Basil\n[ ] Lemons", true, true, null, now - 3 * HOUR, now - 20 * MIN, MACBOOK, "Pankaj’s MacBook Pro", null, null, false),
        NoteItem("n2", "Trip to Lisbon", "Flight lands 10:40. Hotel near Alfama, check-in from 14:00.", false, false, "blue", now - 30 * HOUR, now - 2 * HOUR, ME, MY_NAME, null, now + 26 * HOUR, false),
        NoteItem("n3", "Wi-Fi for guests", "Network: Guest-5G\nAsk at the door for the code.", false, false, null, now - 72 * HOUR, now - 50 * HOUR, MINI, "Home Mac mini", null, null, false),
    )

    fun timers(now: Long): List<TimerItem> = listOf(
        TimerItem("tm1", "Pasta", 10 * MIN, "running", now + 7 * MIN, 0, listOf(MACBOOK), now - 3 * MIN, now - 3 * MIN, MACBOOK, "Pankaj’s MacBook Pro"),
        TimerItem("tm2", "Laundry", 45 * MIN, "paused", 0, 28 * MIN, listOf(MINI), now - 17 * MIN, now - 2 * MIN, MINI, "Home Mac mini"),
    )

    fun readLater(now: Long): List<ReadLaterEntry> = listOf(
        ReadLaterEntry("r1", "https://example.com/talks/local-first-software", "Local-first software", "example.com", false, now - 2 * HOUR, "Home Mac mini", now - 2 * HOUR),
        ReadLaterEntry("r2", "https://example.org/guides/quic", "A gentle guide to QUIC", "example.org", false, now - 26 * HOUR, MY_NAME, now - 26 * HOUR),
        ReadLaterEntry("r3", "https://example.net/recipes/focaccia", "Overnight focaccia", "example.net", true, now - 80 * HOUR, "Pankaj’s MacBook Pro", now - 70 * HOUR),
    )
}
