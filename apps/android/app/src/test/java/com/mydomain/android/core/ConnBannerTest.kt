package com.mydomain.android.core

import com.mydomain.android.core.ConnBanner.State
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class ConnBannerTest {
    @Test
    fun offlineComesFirst() {
        assertEquals(State.OFFLINE, ConnBanner.state(online = false, serverReason = "ok", recovering = false))
        assertEquals(State.OFFLINE, ConnBanner.state(online = false, serverReason = null, recovering = true))
    }

    @Test
    fun backOnlineIsReconnectingUntilTheServerAnswers() {
        assertEquals(State.RECONNECTING, ConnBanner.state(online = true, serverReason = "unreachable", recovering = true))
    }

    @Test
    fun theServerLine() {
        assertEquals(State.SERVER_UNREACHABLE, ConnBanner.state(online = true, serverReason = "unreachable", recovering = false))
        assertNull(ConnBanner.state(online = true, serverReason = "ok", recovering = false))
        // Not checked yet: nothing to say.
        assertNull(ConnBanner.state(online = true, serverReason = null, recovering = false))
    }
}
