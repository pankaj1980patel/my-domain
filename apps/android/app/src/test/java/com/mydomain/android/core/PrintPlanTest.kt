package com.mydomain.android.core

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class PrintPlanTest {
    private val hall = PrintPlan.Remote(
        name = "Hall",
        label = "Hall printer",
        isDefault = true,
        color = true,
        duplex = true,
        papers = listOf("Letter", "A4", "iso_a4_210x297mm", "Custom.4x6in", "B5", "Env10"),
        defaultPaper = "A4",
    )
    private val mono = hall.copy(color = false, duplex = false)

    @Test fun printerIdsNameTheComputerAndThePrinter() {
        val id = PrintPlan.localId("node-1", "Brother HL-L2350DW")
        assertEquals("node-1" to "Brother HL-L2350DW", PrintPlan.parseLocalId(id))
        assertNull(PrintPlan.parseLocalId("no-separator"))
        assertNull(PrintPlan.parseLocalId("\nprinter"))
        assertNull(PrintPlan.parseLocalId("node\n"))
    }

    @Test fun papersMapToAndroidSizes() {
        assertEquals("ISO_A4", PrintPlan.mediaIdFor("A4"))
        assertEquals("ISO_A4", PrintPlan.mediaIdFor("iso_a4_210x297mm"))
        assertEquals("NA_LETTER", PrintPlan.mediaIdFor("Letter"))
        assertEquals("NA_LETTER", PrintPlan.mediaIdFor("na_letter_8.5x11in"))
        assertEquals("NA_LEGAL", PrintPlan.mediaIdFor("Legal"))
        assertEquals("ISO_A3", PrintPlan.mediaIdFor("A3"))
        assertEquals("JIS_B5", PrintPlan.mediaIdFor("B5"))
        assertEquals("ISO_B5", PrintPlan.mediaIdFor("iso_b5_176x250mm"))
        assertEquals("NA_INDEX_4X6", PrintPlan.mediaIdFor("na_index-4x6_4x6in"))
        assertNull(PrintPlan.mediaIdFor("Env10"))
        assertNull(PrintPlan.mediaIdFor("Custom.WIDTHxHEIGHT"))
    }

    @Test fun eachSizeOnceWithThePrintersDefault() {
        val (media, default) = PrintPlan.media(hall)
        assertEquals(listOf("NA_LETTER", "ISO_A4", "JIS_B5"), media.keys.toList())
        assertEquals("A4", media["ISO_A4"])
        assertEquals("ISO_A4", default)
        // Nothing Android knows: A4 and Letter on the printer's own default.
        val (fallback, d) = PrintPlan.media(hall.copy(papers = listOf("Env10"), defaultPaper = null))
        assertEquals(mapOf("ISO_A4" to "", "NA_LETTER" to ""), fallback)
        assertEquals("ISO_A4", d)
    }

    @Test fun theDialogOnlyOffersWhatThePrinterCanDo() {
        assertEquals(
            (PrintPlan.COLOR_MODE_COLOR or PrintPlan.COLOR_MODE_MONOCHROME) to PrintPlan.COLOR_MODE_COLOR,
            PrintPlan.colorModes(hall),
        )
        assertEquals(PrintPlan.COLOR_MODE_MONOCHROME to PrintPlan.COLOR_MODE_MONOCHROME, PrintPlan.colorModes(mono))
        assertEquals(PrintPlan.DUPLEX_MODE_NONE to PrintPlan.DUPLEX_MODE_NONE, PrintPlan.duplexModes(mono))
        assertEquals(7 to PrintPlan.DUPLEX_MODE_NONE, PrintPlan.duplexModes(hall))
    }

    @Test fun choicesBecomeTheJobsOptions() {
        val c = PrintPlan.choice(hall, 3, PrintPlan.COLOR_MODE_MONOCHROME, PrintPlan.DUPLEX_MODE_LONG_EDGE, "ISO_A4", portrait = false)
        assertEquals(PrintPlan.Choice(3, false, "long", "A4", "landscape"), c)
        val s = PrintPlan.choice(hall, 0, PrintPlan.COLOR_MODE_COLOR, PrintPlan.DUPLEX_MODE_SHORT_EDGE, "NA_LETTER", portrait = true)
        assertEquals(PrintPlan.Choice(1, true, "short", "Letter", "portrait"), s)
        // A printer without colour or two sides never gets asked for them.
        val m = PrintPlan.choice(mono, 500, PrintPlan.COLOR_MODE_COLOR, PrintPlan.DUPLEX_MODE_LONG_EDGE, "ISO_A3", portrait = true)
        assertEquals(PrintPlan.Choice(99, false, "none", "", "portrait"), m)
        assertEquals("", PrintPlan.choice(hall, 1, 2, 1, null, true).paper)
    }

    @Test fun computerStatesBecomeJobSteps() {
        assertEquals(PrintPlan.Step.WAITING, PrintPlan.step("waiting"))
        assertEquals(PrintPlan.Step.PRINTING, PrintPlan.step("printing"))
        assertEquals(PrintPlan.Step.DONE, PrintPlan.step("done"))
        assertEquals(PrintPlan.Step.FAILED, PrintPlan.step("failed"))
        assertEquals(PrintPlan.Step.IGNORE, PrintPlan.step("exploded"))
    }

    @Test fun theFileNameIsSafe() {
        assertEquals("Boarding pass.pdf", PrintPlan.fileName("Boarding pass"))
        assertEquals("ticket.PDF", PrintPlan.fileName("ticket.PDF"))
        assertEquals("a_b.pdf", PrintPlan.fileName("a/b"))
        assertEquals("document.pdf", PrintPlan.fileName(null))
        assertEquals("rf.pdf", PrintPlan.fileName("--rf"))
        assertFalse(PrintPlan.fileName("x\n").contains("\n"))
        assertTrue(PrintPlan.fileName("y".repeat(200)).length <= 84)
    }
}
