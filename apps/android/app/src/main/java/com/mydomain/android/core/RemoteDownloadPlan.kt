package com.mydomain.android.core

/**
 * "Download on <device>": a link shared on this phone is downloaded by
 * another of your devices itself, or one of them asks this phone to
 * download one. The core does the downloading and the checks
 * (`features::remote_download`); these are the phone's rules around it, in
 * plain Kotlin so JVM tests cover them ([RemoteDownloads] holds the state).
 */

/** One download, either way, as the core's `remote_download` event and
 *  `remote_downloads` give it. [here]: this phone downloads it, asked by
 *  [node]; else [node] downloads it, asked from here. */
data class DlItem(
    val id: String,
    val node: String,
    val here: Boolean,
    val url: String,
    val host: String,
    val name: String,
    val size: Long = 0,
    val done: Long = 0,
    val state: String,
    val error: String? = null,
    val startedAt: Long = 0,
) {
    val finished: Boolean get() = state in RemoteDownloadPlan.FINISHED

    /** Waiting for the user on this phone. */
    val asking: Boolean get() = here && state == "asking"
}

object RemoteDownloadPlan {
    val FINISHED = setOf("done", "failed", "cancelled", "refused")

    /** The answers to "Downloads from my devices". */
    val POLICIES = listOf("ask", "always", "never")

    /** The size limits offered, in MB (0: no limit). */
    val MAX_CHOICES = listOf(100L, 500L, 1024L, 4096L, 0L)

    /** What an active row says (finished rows say what transfers say). */
    enum class Line {
        /** Here: asking the server about it. */
        Checking,
        /** Here: waiting for the user. */
        Asking,
        /** Here: another download is running. */
        Queued,
        /** Here: bytes coming. */
        Downloading,
        /** There: not answered yet. */
        Requested,
        /** There: the other device asks its user. */
        WaitingThere,
        /** There: queued on the other device. */
        QueuedThere,
        /** There: the other device is downloading. */
        DownloadingThere,
    }

    fun line(item: DlItem): Line = if (item.here) {
        when (item.state) {
            "asking" -> Line.Asking
            "queued" -> Line.Queued
            "downloading" -> Line.Downloading
            else -> Line.Checking
        }
    } else {
        when (item.state) {
            "asking" -> Line.WaitingThere
            "queued" -> Line.QueuedThere
            "downloading" -> Line.DownloadingThere
            else -> Line.Requested
        }
    }

    /** The link to download in a share's [text] (else its [subject]). */
    fun linkIn(text: String?, subject: String? = null): String? =
        ReadLaterLinks.fromShare(text, subject)?.url

    /** The devices a link can go to: those that said they download links,
     *  linked ones first, then by name. */
    fun targets(
        peers: List<String>,
        supports: (String) -> Boolean,
        linked: (String) -> Boolean,
        name: (String) -> String,
    ): List<String> = peers.distinct()
        .filter(supports)
        .sortedWith(compareByDescending<String> { linked(it) }.thenBy { name(it).lowercase() })

    /** Questions waiting here, the oldest first. */
    fun asks(items: Collection<DlItem>): List<DlItem> =
        items.filter { it.asking }.sortedWith(compareBy<DlItem> { it.startedAt }.thenBy { it.id })

    /** The offered limit closest to [mb] (a limit set on another device, or
     *  from a backup, may be none of them). */
    fun nearestMax(mb: Long): Long {
        if (mb <= 0) return 0
        return MAX_CHOICES.filter { it > 0 }.minByOrNull { kotlin.math.abs(it - mb) } ?: 0
    }

    /** A download's transfer id (the core makes them `dl` + hex; a file
     *  transfer's id is hex only). */
    fun isDownloadId(id: String): Boolean = id.startsWith("dl")

    /** A policy as kept: unknown ones read as "ask". */
    fun policy(p: String?): String = p?.takeIf { it in POLICIES } ?: "ask"
}
