package com.mydomain.android

import android.content.Intent
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.print.PrintAttributes
import android.print.PrinterCapabilitiesInfo
import android.print.PrinterId
import android.print.PrinterInfo
import android.printservice.PrintJob
import android.printservice.PrintService
import android.printservice.PrinterDiscoverySession
import android.util.Log
import com.mydomain.android.core.Core
import com.mydomain.android.core.PrintPlan
import org.json.JSONObject
import java.io.File
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.Executors

/**
 * Print from the phone on a computer's printer (mdcore `features::print`):
 * an Android print service, so "my-domain" shows in every app's print
 * dialog with the printers of each of your computers that print for your
 * phones (they say `print` in their status; nothing is sent to one that
 * doesn't).
 *
 *  - Discovery asks each such computer for its printers (the `printers`
 *    event answers) and lists them with what each can do: paper sizes
 *    (PrintPlan maps the printer's names to Android's), colour, two-sided.
 *  - A job's PDF (Android has applied the page range already) is copied
 *    out of the dialog and sent to the computer as an ordinary file
 *    transfer tagged as a print job, with the copies, colour, two-sided,
 *    paper and orientation chosen. The copy is deleted once sent or failed.
 *  - Android's print notification follows the job: sending (with
 *    progress), waiting for the computer, printing, then done or failed
 *    with the computer's reason. Cancel stops the transfer.
 *
 * The engine lives in NetService; it's started if the app wasn't running.
 * Core events arrive through [onEvent] (from NetService).
 */
class PrintToComputer : PrintService() {
    override fun onCreate() {
        super.onCreate()
        Core.init(this)
        ensureEngine()
    }

    override fun onConnected() {
        instance = this
    }

    override fun onDisconnected() {
        if (instance === this) instance = null
    }

    override fun onCreatePrinterDiscoverySession(): PrinterDiscoverySession = Discovery()

    private fun ensureEngine() {
        if (Core.engine() != null) return
        val service = Intent(this, NetService::class.java)
        runCatching { if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) startForegroundService(service) else startService(service) }
    }

    // --- discovery ---------------------------------------------------------------

    private inner class Discovery : PrinterDiscoverySession() {
        override fun onStartPrinterDiscovery(priorityList: MutableList<PrinterId>) {
            sessions.add(this)
            ensureEngine()
            publishAll()
            askComputers()
        }

        override fun onStopPrinterDiscovery() {
            sessions.remove(this)
        }

        override fun onValidatePrinters(printerIds: MutableList<PrinterId>) {}

        override fun onStartPrinterStateTracking(printerId: PrinterId) {
            // Look again: the printer may have gone (or come) since.
            PrintPlan.parseLocalId(printerId.localId)?.let { (node, _) -> ask(node) }
        }

        override fun onStopPrinterStateTracking(printerId: PrinterId) {}

        override fun onDestroy() {
            sessions.remove(this)
        }

        /** Every known printer of the computers that print for your phones. */
        fun publishAll() {
            val engine = Core.engine()
            for ((node, list) in known) {
                if (engine?.printSupported(node) == false) continue
                publish(node, list)
            }
        }

        /** [node]'s printers now: the ones it no longer has go. */
        fun publish(node: String, list: List<PrintPlan.Remote>) {
            val now = list.map { info(node, it) }
            val keep = now.map { it.id }.toSet()
            val gone = getPrinters().filter { PrintPlan.parseLocalId(it.id.localId)?.first == node && it.id !in keep }.map { it.id }
            if (gone.isNotEmpty()) removePrinters(gone)
            if (now.isNotEmpty()) addPrinters(now)
        }

        private fun askComputers() {
            val engine = Core.engine() ?: return
            io.execute {
                val nodes = runCatching { engine.peers() }.getOrDefault(emptyList())
                    .map { it.nodeId }
                    .filter { runCatching { engine.printSupported(it) }.getOrDefault(false) }
                nodes.forEach { ask(it) }
            }
        }
    }

    private fun ask(node: String) {
        val engine = Core.engine() ?: return
        io.execute {
            runCatching { engine.requestPrinters(node) }
                .onFailure { Log.w(TAG, "asking $node for printers: ${it.message}") }
        }
    }

    private fun info(node: String, p: PrintPlan.Remote): PrinterInfo {
        val id = generatePrinterId(PrintPlan.localId(node, p.name))
        val caps = PrinterCapabilitiesInfo.Builder(id)
        val (media, defaultMedia) = PrintPlan.media(p)
        for (mediaId in media.keys) {
            val size = MEDIA[mediaId] ?: continue
            caps.addMediaSize(size, mediaId == defaultMedia)
        }
        if (media.keys.none { it in MEDIA }) caps.addMediaSize(PrintAttributes.MediaSize.ISO_A4, true)
        caps.addResolution(PrintAttributes.Resolution("default", getString(R.string.print_resolution), 600, 600), true)
        caps.setMinMargins(PrintAttributes.Margins.NO_MARGINS)
        val (colors, color) = PrintPlan.colorModes(p)
        caps.setColorModes(colors, color)
        val (duplexes, duplex) = PrintPlan.duplexModes(p)
        caps.setDuplexModes(duplexes, duplex)
        val computer = Core.peerName(node)
        val shown = p.label.ifBlank { p.name }
        return PrinterInfo.Builder(id, shown, PrinterInfo.STATUS_IDLE)
            .setDescription(getString(R.string.print_on_computer, computer))
            .setCapabilities(caps.build())
            .build()
    }

    // --- jobs --------------------------------------------------------------------

    override fun onPrintJobQueued(job: PrintJob) {
        val target = job.info.printerId?.localId?.let { PrintPlan.parseLocalId(it) }
        val remote = target?.let { (node, name) -> known[node]?.firstOrNull { it.name == name } }
        val engine = Core.engine()
        if (target == null || remote == null || engine == null || !engine.printSupported(target.first)) {
            job.fail(getString(R.string.print_no_computer))
            return
        }
        val (node, _) = target
        val computer = Core.peerName(node)
        if (!job.start()) return
        val attrs = job.info.attributes
        val choice = PrintPlan.choice(
            remote,
            copies = job.info.copies,
            colorMode = attrs.colorMode,
            duplexMode = attrs.duplexMode,
            mediaId = attrs.mediaSize?.id,
            portrait = attrs.mediaSize?.isPortrait ?: true,
        )
        val title = job.document.info.name
        val data = job.document.data
        if (data == null) {
            job.fail(getString(R.string.print_failed_read))
            return
        }
        job.setStatus(getString(R.string.print_sending, computer))
        val dir = File(cacheDir, "print").apply { mkdirs() }
        val file = File(dir, "${System.currentTimeMillis()}-${PrintPlan.fileName(title)}")
        io.execute {
            val copied = runCatching {
                android.os.ParcelFileDescriptor.AutoCloseInputStream(data).use { input ->
                    file.outputStream().use { input.copyTo(it) }
                }
            }
            val sent = copied.mapCatching {
                engine.printFile(
                    node, file.absolutePath, remote.name, choice.copies.toUInt(), choice.color,
                    choice.duplex, choice.paper, choice.orientation, title,
                )
            }
            main.post {
                sent.onSuccess { id ->
                    jobs[id] = Job(job, computer, file)
                }.onFailure {
                    file.delete()
                    Log.w(TAG, "print job not sent: ${it.message}")
                    end(job) { job.fail(getString(R.string.print_failed_send, computer)) }
                }
            }
        }
    }

    override fun onRequestCancelPrintJob(job: PrintJob) {
        val id = jobs.entries.firstOrNull { it.value.job.id == job.id }?.key
        if (id != null) {
            jobs.remove(id)?.file?.delete()
            val engine = Core.engine()
            io.execute { runCatching { engine?.cancelFile(id) } }
        }
        end(job) { job.cancel() }
    }

    private class Job(val job: PrintJob, val computer: String, val file: File)

    companion object {
        private const val TAG = "PrintToComputer"
        @Volatile private var instance: PrintToComputer? = null
        private val main = Handler(Looper.getMainLooper())
        private val io = Executors.newSingleThreadExecutor()
        /** Each computer's printers, as it last said. */
        private val known = ConcurrentHashMap<String, List<PrintPlan.Remote>>()
        /** Discovery sessions open now (main thread). */
        private val sessions = mutableSetOf<Discovery>()
        /** Jobs on their way, by transfer id (main thread). */
        private val jobs = HashMap<String, Job>()

        /** Android's media sizes by id. */
        private val MEDIA: Map<String, PrintAttributes.MediaSize> = listOf(
            PrintAttributes.MediaSize.ISO_A0, PrintAttributes.MediaSize.ISO_A1, PrintAttributes.MediaSize.ISO_A2,
            PrintAttributes.MediaSize.ISO_A3, PrintAttributes.MediaSize.ISO_A4, PrintAttributes.MediaSize.ISO_A5,
            PrintAttributes.MediaSize.ISO_A6, PrintAttributes.MediaSize.ISO_A7, PrintAttributes.MediaSize.ISO_A8,
            PrintAttributes.MediaSize.ISO_B4, PrintAttributes.MediaSize.ISO_B5, PrintAttributes.MediaSize.ISO_B6,
            PrintAttributes.MediaSize.JIS_B4, PrintAttributes.MediaSize.JIS_B5, PrintAttributes.MediaSize.JIS_B6,
            PrintAttributes.MediaSize.NA_LETTER, PrintAttributes.MediaSize.NA_LEGAL, PrintAttributes.MediaSize.NA_TABLOID,
            PrintAttributes.MediaSize.NA_LEDGER, PrintAttributes.MediaSize.NA_GOVT_LETTER,
            PrintAttributes.MediaSize.NA_JUNIOR_LEGAL, PrintAttributes.MediaSize.NA_INDEX_4X6,
            PrintAttributes.MediaSize.NA_INDEX_5X8, PrintAttributes.MediaSize.JPN_HAGAKI,
        ).associateBy { it.id }

        /** Only while the job is still going: calls on an ended one throw. */
        private fun end(job: PrintJob, f: () -> Unit) {
            if (job.isCancelled || job.isCompleted || job.isFailed) return
            runCatching { f() }.onFailure { Log.w(TAG, "print job: ${it.message}") }
        }

        /** Core events (NetService, any thread). */
        fun onEvent(o: JSONObject) {
            when (o.optString("kind")) {
                "printers" -> {
                    val from = o.optString("from")
                    val list = if (o.optBoolean("on")) parse(o) else emptyList()
                    if (list.isEmpty()) known.remove(from) else known[from] = list
                    main.post { sessions.forEach { it.publish(from, list) } }
                }
                "print_status" -> main.post { onStatus(o) }
                "file_progress" -> if (!o.optBoolean("incoming")) main.post {
                    val j = jobs[o.optString("id")] ?: return@post
                    val total = o.optLong("total")
                    if (total > 0) end(j.job) { j.job.setProgress(o.optLong("done").toFloat() / total) }
                }
                "file_sent" -> main.post {
                    val j = jobs[o.optString("id")] ?: return@post
                    j.file.delete()
                    val svc = instance ?: return@post
                    end(j.job) { j.job.setStatus(svc.getString(R.string.print_sent, j.computer)) }
                }
                "file_failed" -> if (!o.optBoolean("incoming")) main.post {
                    val j = jobs.remove(o.optString("id")) ?: return@post
                    j.file.delete()
                    val why = o.optString("reason").ifBlank { instance?.getString(R.string.print_failed_send, j.computer) ?: "" }
                    end(j.job) { j.job.fail(why) }
                }
            }
        }

        private fun onStatus(o: JSONObject) {
            val id = o.optString("id")
            val j = jobs[id] ?: return
            val svc = instance
            when (PrintPlan.step(o.optString("state"))) {
                PrintPlan.Step.WAITING -> end(j.job) { j.job.setStatus(svc?.getString(R.string.print_waiting, j.computer)) }
                PrintPlan.Step.PRINTING -> end(j.job) { j.job.setStatus(svc?.getString(R.string.print_printing, j.computer)) }
                PrintPlan.Step.DONE -> {
                    jobs.remove(id)?.file?.delete()
                    end(j.job) { j.job.complete() }
                }
                PrintPlan.Step.FAILED -> {
                    jobs.remove(id)?.file?.delete()
                    val why = o.optString("reason").ifBlank { svc?.getString(R.string.print_failed_computer, j.computer) ?: "" }
                    end(j.job) { j.job.fail(why) }
                }
                PrintPlan.Step.IGNORE -> {}
            }
        }

        private fun parse(o: JSONObject): List<PrintPlan.Remote> {
            val arr = o.optJSONArray("printers") ?: return emptyList()
            return (0 until arr.length()).mapNotNull { i ->
                val p = arr.optJSONObject(i) ?: return@mapNotNull null
                val name = p.optString("name").ifEmpty { return@mapNotNull null }
                val papers = p.optJSONArray("papers")?.let { a -> (0 until a.length()).map { a.optString(it) } }.orEmpty()
                PrintPlan.Remote(
                    name = name,
                    label = p.optString("label"),
                    isDefault = p.optBoolean("is_default"),
                    color = p.optBoolean("color"),
                    duplex = p.optBoolean("duplex"),
                    papers = papers.filter { it.isNotEmpty() },
                    defaultPaper = p.optString("default_paper").takeIf { it.isNotEmpty() && !p.isNull("default_paper") },
                )
            }
        }
    }
}
