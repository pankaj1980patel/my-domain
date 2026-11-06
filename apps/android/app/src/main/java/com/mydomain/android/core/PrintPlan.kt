package com.mydomain.android.core

/**
 * Print from the phone on a computer's printer (PrintToComputer.kt): how
 * Android's print dialog and a computer's printers meet. Pure Kotlin (no
 * Android types), so the JVM tests cover it:
 *  - each printer's id in the dialog names its computer and its name there;
 *  - the paper sizes a printer lists (CUPS names: "A4", "Letter",
 *    "iso_a4_210x297mm", ...) map to Android's media sizes (by their ids:
 *    "ISO_A4", "NA_LETTER", ...) and back, so the job names the printer's
 *    own paper;
 *  - the dialog's colour, two-sided, copies and orientation become the
 *    job's options, never asking for what the printer can't do;
 *  - the computer's job states become the print job's.
 * The constants are Android's own values (PrintAttributes), copied so this
 * stays plain Kotlin.
 */
object PrintPlan {
    const val COLOR_MODE_MONOCHROME = 1
    const val COLOR_MODE_COLOR = 2
    const val DUPLEX_MODE_NONE = 1
    const val DUPLEX_MODE_LONG_EDGE = 2
    const val DUPLEX_MODE_SHORT_EDGE = 4
    const val MAX_COPIES = 99

    /** One of a computer's printers, as its `printers` event says. */
    data class Remote(
        val name: String,
        val label: String,
        val isDefault: Boolean,
        val color: Boolean,
        val duplex: Boolean,
        val papers: List<String>,
        val defaultPaper: String?,
    )

    /** What the job asks the computer for. */
    data class Choice(
        val copies: Int,
        val color: Boolean,
        val duplex: String,
        val paper: String,
        val orientation: String,
    )

    // --- printer ids -------------------------------------------------------------

    private const val SEP = '\n'

    /** The dialog's local id for [printer] on computer [node]. */
    fun localId(node: String, printer: String): String = "$node$SEP$printer"

    /** The computer and printer a local id names, or null. */
    fun parseLocalId(id: String): Pair<String, String>? {
        val at = id.indexOf(SEP)
        if (at <= 0 || at == id.length - 1) return null
        return id.substring(0, at) to id.substring(at + 1)
    }

    // --- paper -------------------------------------------------------------------

    /** Android's media size id for a printer's paper name, or null if
     *  Android has none like it. */
    fun mediaIdFor(paper: String): String? {
        val p = paper.lowercase()
        // Driverless names: "iso_a4_210x297mm", "na_letter_8.5x11in".
        val parts = p.split('_')
        val (family, size) = when {
            parts.size >= 2 && parts[0] in setOf("iso", "na", "jis", "jpn") -> parts[0] to parts[1]
            else -> "" to p
        }
        return when (size) {
            "a0", "a1", "a2", "a3", "a4", "a5", "a6", "a7", "a8" -> "ISO_${size.uppercase()}"
            // A PPD's bare "B5" is JIS B5 (182 x 257 mm), as CUPS defines it.
            "b4", "b5", "b6" -> if (family == "iso") "ISO_${size.uppercase()}" else "JIS_${size.uppercase()}"
            "letter", "lettersmall" -> "NA_LETTER"
            "legal" -> "NA_LEGAL"
            "tabloid", "11x17" -> "NA_TABLOID"
            "ledger" -> "NA_LEDGER"
            "govt-letter" -> "NA_GOVT_LETTER"
            "junior-legal" -> "NA_JUNIOR_LEGAL"
            "index-4x6", "4x6", "4x6in", "photo4x6" -> "NA_INDEX_4X6"
            "index-5x8", "5x8" -> "NA_INDEX_5X8"
            "hagaki", "postcard" -> "JPN_HAGAKI"
            else -> when {
                p.startsWith("jisb5") -> "JIS_B5"
                p.startsWith("jisb4") -> "JIS_B4"
                p.startsWith("4x6") -> "NA_INDEX_4X6"
                else -> null
            }
        }
    }

    /** The media sizes to offer for [printer]: Android media id -> the
     *  printer's paper name, in the printer's order (the first paper of
     *  each size wins), and which is the default. With no paper Android
     *  knows, A4 and Letter with the printer's own default paper ("").
     */
    fun media(printer: Remote): Pair<LinkedHashMap<String, String>, String> {
        val out = LinkedHashMap<String, String>()
        for (p in printer.papers) {
            val id = mediaIdFor(p) ?: continue
            if (id !in out) out[id] = p
        }
        if (out.isEmpty()) {
            out["ISO_A4"] = ""
            out["NA_LETTER"] = ""
            return out to "ISO_A4"
        }
        val default = printer.defaultPaper?.let { mediaIdFor(it) }?.takeIf { it in out } ?: out.keys.first()
        return out to default
    }

    // --- the job -----------------------------------------------------------------

    /** Colour modes for the dialog: (mask, default). */
    fun colorModes(printer: Remote): Pair<Int, Int> =
        if (printer.color) (COLOR_MODE_COLOR or COLOR_MODE_MONOCHROME) to COLOR_MODE_COLOR
        else COLOR_MODE_MONOCHROME to COLOR_MODE_MONOCHROME

    /** Two-sided modes for the dialog: (mask, default). */
    fun duplexModes(printer: Remote): Pair<Int, Int> =
        if (printer.duplex) (DUPLEX_MODE_NONE or DUPLEX_MODE_LONG_EDGE or DUPLEX_MODE_SHORT_EDGE) to DUPLEX_MODE_NONE
        else DUPLEX_MODE_NONE to DUPLEX_MODE_NONE

    /** The job's options from the dialog's choices ([mediaId] null: the
     *  printer's default paper). */
    fun choice(
        printer: Remote,
        copies: Int,
        colorMode: Int,
        duplexMode: Int,
        mediaId: String?,
        portrait: Boolean,
    ): Choice {
        val color = printer.color && colorMode != COLOR_MODE_MONOCHROME
        val duplex = if (!printer.duplex) "none" else when (duplexMode) {
            DUPLEX_MODE_LONG_EDGE -> "long"
            DUPLEX_MODE_SHORT_EDGE -> "short"
            else -> "none"
        }
        val paper = mediaId?.let { media(printer).first[it] } ?: ""
        return Choice(
            copies = copies.coerceIn(1, MAX_COPIES),
            color = color,
            duplex = duplex,
            paper = paper,
            orientation = if (portrait) "portrait" else "landscape",
        )
    }

    /** What the print job does on a computer's state. */
    enum class Step { WAITING, PRINTING, DONE, FAILED, IGNORE }

    fun step(state: String): Step = when (state) {
        "waiting" -> Step.WAITING
        "printing" -> Step.PRINTING
        "done" -> Step.DONE
        "failed" -> Step.FAILED
        else -> Step.IGNORE
    }

    /** A file name for the job's PDF, from the document's name. */
    fun fileName(document: String?): String {
        val base = (document ?: "").replace(Regex("[\\\\/:*?\"<>|\\p{Cntrl}]"), "_").trim().trimStart('.', '-')
        val name = base.ifEmpty { "document" }.take(80)
        return if (name.lowercase().endsWith(".pdf")) name else "$name.pdf"
    }
}
