package com.mydomain.android.core

/**
 * The app-wide connection line (the pure part, unit-tested; the banner is
 * ui/StatusBanner.kt): what to say, from real state only.
 */
object ConnBanner {
    enum class State { OFFLINE, RECONNECTING, SERVER_UNREACHABLE }

    /**
     * [online]: the phone has a network with internet. [serverReason]: the
     * health check's last "server" line ("ok", or why not; null: not
     * checked yet). [recovering]: the network came back and the server
     * hasn't been asked again since. Null: nothing to say.
     */
    fun state(online: Boolean, serverReason: String?, recovering: Boolean): State? = when {
        !online -> State.OFFLINE
        recovering -> State.RECONNECTING
        serverReason != null && serverReason != "ok" -> State.SERVER_UNREACHABLE
        else -> null
    }
}
