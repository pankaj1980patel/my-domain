package com.mydomain.android.core

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
}
