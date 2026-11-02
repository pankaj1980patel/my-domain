package com.mydomain.android.core

import java.net.Inet4Address
import java.net.NetworkInterface

object NetworkingUtils {
    /** This device's Wi-Fi IPv4 (any site-local IPv4 as a fallback), or "". */
    fun wifiIpv4(): String {
        fun scan(preferWlan: Boolean): String? {
            val ifaces = runCatching { NetworkInterface.getNetworkInterfaces() }.getOrNull() ?: return null
            for (iface in ifaces) {
                if (!iface.isUp || iface.isLoopback) continue
                val name = iface.name.lowercase()
                if (preferWlan && !(name.startsWith("wlan") || name.startsWith("ap"))) continue
                for (addr in iface.inetAddresses) {
                    if (addr is Inet4Address && !addr.isLoopbackAddress) {
                        val host = addr.hostAddress
                        if (host != null && (preferWlan || addr.isSiteLocalAddress)) return host
                    }
                }
            }
            return null
        }
        return scan(true) ?: scan(false) ?: ""
    }
}
