package com.mydomain.android.core

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class JoinLinksTest {

    // As mdcore's JoinLink::to_uri writes them.
    private val link = "mydomain://join?v=1&s=https://mydomain.servehttp.com&u=ann&t=AAECAwQFBgcICQoLDA0ODw&k=c2VhbGVkLWtleQ"

    @Test
    fun readsTheAccountAndServer() {
        assertEquals(JoinLinks.Link("https://mydomain.servehttp.com", "ann"), JoinLinks.parse(link))
        assertEquals(JoinLinks.Link("https://mydomain.servehttp.com", "ann"), JoinLinks.parse("  $link\n"))
        assertTrue(JoinLinks.isJoinLink(link.replace("mydomain://join", "MYDOMAIN://JOIN")))
    }

    @Test
    fun decodesEscapedValues() {
        val l = "mydomain://join?v=1&s=http://10.0.0.5:6969&u=ann%20%26%20bob%3Dok%2B%25%C3%A9&t=x&k=y"
        assertEquals(JoinLinks.Link("http://10.0.0.5:6969", "ann & bob=ok+%é"), JoinLinks.parse(l))
    }

    @Test
    fun refusesOtherLinks() {
        assertNull(JoinLinks.parse("https://mydomain.servehttp.com/join?v=1"))
        assertNull(JoinLinks.parse(link.replace("v=1", "v=2")))
        assertNull(JoinLinks.parse(link.replace("s=https://", "s=ftp://")))
        assertNull(JoinLinks.parse(link.replace("&u=ann", "")))
        assertNull(JoinLinks.parse(link.replace("&k=c2VhbGVkLWtleQ", "")))
        assertNull(JoinLinks.parse("mydomain://join?v=1&s=%ZZ"))
        assertNull(JoinLinks.parse("mydomain://join?garbage"))
        assertFalse(JoinLinks.isJoinLink("123456"))
    }

    @Test
    fun typedCodesAreForgiving() {
        assertEquals("123456", JoinLinks.codeDigits("123456"))
        assertEquals("123456", JoinLinks.codeDigits(" 123 456 "))
        assertEquals("123456", JoinLinks.codeDigits("123-456"))
        assertNull(JoinLinks.codeDigits("12345"))
        assertNull(JoinLinks.codeDigits("1234567"))
        assertNull(JoinLinks.codeDigits("12a456"))
        assertNull(JoinLinks.codeDigits(""))
    }

    @Test
    fun formatsForTheScreen() {
        assertEquals("123 456", JoinLinks.spaced("123456"))
        assertEquals("12", JoinLinks.spaced("12"))
        assertEquals("4:59", JoinLinks.mmss(299))
        assertEquals("0:05", JoinLinks.mmss(5))
        assertEquals("0:00", JoinLinks.mmss(-3))
    }
}
