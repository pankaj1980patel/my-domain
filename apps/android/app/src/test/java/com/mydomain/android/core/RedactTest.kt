package com.mydomain.android.core

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class RedactTest {
    private val h = Redact.HIDDEN

    @Test
    fun aCodeEventKeepsNoCode() {
        val out = Redact.event("verification-code", """{"kind":"verification_code","code":"482913","app":"Bank","text":"Your code is 482913"}""")
        assertFalse(out, out.contains("482913"))
        assertTrue(out.contains(""""kind":"verification_code""""))
        assertTrue(out.contains(""""app":"Bank""""))
    }

    @Test
    fun passwordsKeysAndTokensGoWherever() {
        val out = Redact.event(
            "anything",
            """{"kind":"x","password":"hunter2","passphrase" : "a b c","token":"eyJ.x","key":"k\"q","pin":1234,"name":"Pixel"}""",
        )
        listOf("hunter2", "a b c", "eyJ.x", "k\\\"q", "1234").forEach { assertFalse(it, out.contains(it)) }
        assertTrue(out.contains(""""name":"Pixel""""))
        assertTrue(out.contains(""""pin":"$h""""))
    }

    @Test
    fun aTextSentAsAPasswordIsHidden() {
        val out = Redact.event("text-shared", """{"kind":"text_shared","text":"s3cret!","mode":"clipboard","sensitive":true}""")
        assertFalse(out.contains("s3cret!"))
        val plain = Redact.event("text-shared", """{"kind":"text_shared","text":"see you at 5","sensitive":false}""")
        assertTrue(plain.contains("see you at 5"))
    }

    @Test
    fun clipboardTextIsHidden() {
        val out = Redact.event("clipboard-event", """{"kind":"clipboard","content":"my password"}""")
        assertFalse(out.contains("my password"))
        val msg = Redact.event(
            "message-received",
            """{"kind":"message_received","text":"{\"type\":\"clipboard\",\"content\":\"pw123\"}"}""",
        )
        assertFalse(msg, msg.contains("pw123"))
    }

    @Test
    fun aCodeInANotificationIsMaskedButTheTextStays() {
        val out = Redact.event("notification-event", """{"kind":"notification","title":"Bank","body":"Use 739104 to sign in"}""")
        assertFalse(out.contains("739104"))
        assertTrue(out, out.contains("Use $h to sign in"))
        assertTrue(out.contains(""""title":"Bank""""))
    }

    @Test
    fun ordinaryEventsAreUnchanged() {
        val p = """{"kind":"peer_connected","node_id":"abc123","transport":"quic","rtt_ms":12345}"""
        assertEquals(p, Redact.event("peer-connected", p))
    }

    @Test
    fun brokenJsonIsStillCleaned() {
        val out = Redact.event("x", """{"password":"open sesame", "oops""")
        assertFalse(out.contains("open sesame"))
    }
}
