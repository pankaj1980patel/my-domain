package com.mydomain.android.core

import com.mydomain.android.R
import org.junit.Assert.assertEquals
import org.junit.Test

class ErrorWordsTest {
    @Test
    fun knownErrorsGetTheirOwnWords() {
        assertEquals(R.string.error_wrong_password, ErrorWords.resOf("Incorrect password"))
        assertEquals(R.string.error_bad_credentials, ErrorWords.resOf("registry: HTTP 401 Unauthorized"))
        assertEquals(R.string.error_username_taken, ErrorWords.resOf("user already exists"))
        assertEquals(R.string.error_timeout, ErrorWords.resOf("operation timed out"))
        assertEquals(R.string.error_unreachable, ErrorWords.resOf("dns error: failed to lookup address"))
        assertEquals(R.string.error_join_code_used, ErrorWords.resOf("that code expired or was already used"))
        assertEquals(
            R.string.error_join_no_key,
            ErrorWords.resOf("the code worked but the device key didn't come through: sign in with your password instead"),
        )
        assertEquals(R.string.signin_join_missing, ErrorWords.resOf("invalid input: type the 6-digit code"))
    }

    @Test
    fun anythingElseIsThePlainFallbackNeverTheRawText() {
        for (raw in listOf(
            "",
            "called `Option::unwrap()` on a `None` value",
            "quinn: connection lost: ApplicationClosed { error_code: 0 }",
            "failed to derive key",
            "serde_json: expected value at line 1 column 1",
        )) {
            assertEquals(raw, ErrorWords.FALLBACK, ErrorWords.resOf(raw))
        }
    }
}
