package com.mydomain.android.core

import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import java.io.File
import java.nio.file.Files

class HistoryFileTest {
    private val dir: File = Files.createTempDirectory("history").toFile()
    private val f = File(dir, "chat-history.json")
    private val logged = mutableListOf<String>()

    @After
    fun cleanUp() {
        dir.deleteRecursively()
    }

    private fun read() = HistoryFile.read(f, HistoryFile::isJsonList) { logged += it }

    @Test
    fun noFileIsNoHistory() {
        assertNull(read())
        assertTrue(logged.isEmpty())
    }

    @Test
    fun aGoodFileIsRead() {
        f.writeText("""[{"id":"1"}]""")
        assertEquals("""[{"id":"1"}]""", read())
        assertTrue(f.exists())
        assertTrue(logged.isEmpty())
    }

    @Test
    fun aCorruptFileIsMovedAsideNotLost() {
        f.writeText("""[{"id":"1","text":"hel""")
        assertNull(read())
        assertFalse("not left to be overwritten", f.exists())
        val aside = File(dir, "chat-history.json.corrupt")
        assertEquals("""[{"id":"1","text":"hel""", aside.readText())
        assertEquals(1, logged.size)
        assertTrue(logged[0], logged[0].contains("chat-history.json.corrupt"))
    }

    @Test
    fun theWrongShapeIsCorruptToo() {
        f.writeText("""{"not":"a list"}""")
        assertNull(read())
        assertTrue(File(dir, "chat-history.json.corrupt").exists())
    }

    @Test
    fun anEarlierCorruptCopyIsNeverOverwritten() {
        f.writeText("first bad")
        read()
        f.writeText("second bad")
        read()
        assertEquals("first bad", File(dir, "chat-history.json.corrupt").readText())
        assertEquals("second bad", File(dir, "chat-history.json.corrupt-2").readText())
    }

    @Test
    fun theCopiesAreCapped() {
        repeat(HistoryFile.MAX_CORRUPT + 3) { i ->
            f.writeText("bad $i")
            read()
        }
        val copies = dir.listFiles()!!.filter { it.name.contains(".corrupt") }
        assertEquals(HistoryFile.MAX_CORRUPT, copies.size)
        assertEquals("the first is kept", "bad 0", File(dir, "chat-history.json.corrupt").readText())
    }

    @Test
    fun anEmptyFileIsNoHistoryAndLeftAlone() {
        f.writeText("")
        assertNull(read())
        assertTrue(logged.isEmpty())
    }
}
