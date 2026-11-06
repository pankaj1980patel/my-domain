package com.mydomain.android.core

/**
 * The device icons as path data on a 24-unit grid, the same drawings as the
 * desktop's `DeviceIcon.tsx`: the shape of the device (phone, laptop, iMac,
 * tower...) with a small OS mark inside (an apple for a Mac, four panes for
 * Windows, a penguin for Linux, a robot head for Android). The marks are
 * simple original glyphs, not the vendors' logos.
 *
 * Pure Kotlin (tested on the JVM); `ui/DeviceLabelUi.kt` draws them in
 * Compose and `ShareTargets.kt` on a bitmap. Names are the core's
 * (`features::device_kind::ICONS`).
 */
object DeviceIconShapes {
    /** One stroke (1.8 units; [thin]: 0.9) or, with [fill], one filled area. */
    data class Part(val d: String, val fill: Boolean = false, val dashed: Boolean = false, val thin: Boolean = false)

    /** An OS mark about 5 units across, centred on ([x], [y]). */
    data class Mark(val kind: String, val x: Float, val y: Float, val scale: Float = 1f)

    data class Shape(val parts: List<Part>, val mark: Mark? = null)

    /** Every icon drawn (same set and order as the core's `ICONS`). */
    val ICONS = listOf(
        "android_phone", "android_tablet", "android_foldable", "phone", "tablet",
        "mac_laptop", "imac", "mac_desktop", "win_laptop", "win_desktop",
        "linux_laptop", "linux_desktop", "laptop", "desktop", "tv", "watch", "unknown",
    )

    private fun n(v: Float): String = if (v == v.toInt().toFloat()) v.toInt().toString() else v.toString()

    /** A rounded rectangle as path data. */
    fun rr(x: Float, y: Float, w: Float, h: Float, r: Float): String {
        val a = "A${n(r)} ${n(r)} 0 0 1"
        return "M${n(x + r)} ${n(y)}H${n(x + w - r)}$a ${n(x + w)} ${n(y + r)}V${n(y + h - r)}" +
            "$a ${n(x + w - r)} ${n(y + h)}H${n(x + r)}$a ${n(x)} ${n(y + h - r)}V${n(y + r)}$a ${n(x + r)} ${n(y)}Z"
    }

    /** A circle as path data. */
    fun circle(cx: Float, cy: Float, r: Float): String =
        "M${n(cx - r)} ${n(cy)}A${n(r)} ${n(r)} 0 1 0 ${n(cx + r)} ${n(cy)}A${n(r)} ${n(r)} 0 1 0 ${n(cx - r)} ${n(cy)}Z"

    /** The parts of an OS mark, centred on (0, 0). */
    fun markParts(kind: String): List<Part> = when (kind) {
        "apple" -> listOf(
            Part(
                "M0 -1.3C-0.8 -1.9 -2.1 -1.7 -2.1 -0.1C-2.1 1.2 -1.1 2.5 -0.6 2.5C-0.3 2.5 -0.2 2.3 0 2.3" +
                    "C0.2 2.3 0.3 2.5 0.6 2.5C1.1 2.5 2.1 1.2 2.1 -0.1C2.1 -1.7 0.8 -1.9 0 -1.3Z",
                fill = true,
            ),
            Part("M0 -1.5C0 -2.3 0.5 -2.8 1.1 -2.9", thin = true),
        )
        "windows" -> listOf(
            Part(rr(-2f, -2f, 1.75f, 1.75f, 0.2f), fill = true),
            Part(rr(0.25f, -2f, 1.75f, 1.75f, 0.2f), fill = true),
            Part(rr(-2f, 0.25f, 1.75f, 1.75f, 0.2f), fill = true),
            Part(rr(0.25f, 0.25f, 1.75f, 1.75f, 0.2f), fill = true),
        )
        "tux" -> listOf(
            Part(circle(0f, -1.6f, 1.1f), fill = true),
            // The belly is a hole (wound the other way).
            Part("M-1.8 2.3C-1.8 0.4 -1.1 -0.8 0 -0.8S1.8 0.4 1.8 2.3ZM0 0C-0.7 0 -1 0.9 -0.95 2H0.95C1 0.9 0.7 0 0 0Z", fill = true),
            Part("M-1.6 0.2L-2.3 1.3M1.6 0.2L2.3 1.3M-1.5 2.6H-0.4M0.4 2.6H1.5", thin = true),
        )
        "android" -> listOf(
            // The eyes are holes (wound the other way).
            Part(
                "M-2.5 1.7A2.5 2.5 0 0 1 2.5 1.7ZM-1.25 0.75A0.35 0.35 0 1 0 -0.55 0.75A0.35 0.35 0 1 0 -1.25 0.75ZM0.55 0.75A0.35 0.35 0 1 0 1.25 0.75A0.35 0.35 0 1 0 0.55 0.75Z",
                fill = true,
            ),
            Part("M-1.3 -0.5L-1.9 -1.5M1.3 -0.5L1.9 -1.5", thin = true),
        )
        else -> emptyList()
    }

    private fun laptop(mac: Boolean) = listOf(
        Part(rr(4f, 5f, 16f, 11f, if (mac) 2f else 1.2f)),
        Part(if (mac) "M2 18.5H22M10.5 18.5V18.1H13.5V18.5" else "M2 19H22"),
    )

    private val monitor = listOf(Part(rr(3f, 4f, 18f, 12f, 1.5f)), Part("M8 20H16M12 16V20"))

    /** How [icon] is drawn; an unknown name draws "unknown". */
    fun shape(icon: String): Shape = when (icon) {
        "android_phone" -> Shape(listOf(Part(rr(6.5f, 2f, 11f, 20f, 2.2f)), Part("M11 18.5H13")), Mark("android", 12f, 10.5f))
        "phone" -> Shape(listOf(Part(rr(7f, 2f, 10f, 20f, 2f)), Part("M11 18H13")))
        "android_tablet" -> Shape(listOf(Part(rr(3.5f, 3f, 17f, 18f, 2f)), Part("M11 18H13")), Mark("android", 12f, 11f))
        "tablet" -> Shape(listOf(Part(rr(4f, 2f, 16f, 20f, 2f)), Part("M11 18H13")))
        "android_foldable" -> Shape(
            listOf(Part(rr(2.5f, 4f, 19f, 16f, 2f)), Part("M12 4V20", dashed = true)),
            Mark("android", 17f, 12f, 0.8f),
        )
        "mac_laptop" -> Shape(laptop(true), Mark("apple", 12f, 10.6f))
        "win_laptop" -> Shape(laptop(false), Mark("windows", 12f, 10.5f))
        "linux_laptop" -> Shape(laptop(false), Mark("tux", 12f, 10.5f))
        "laptop" -> Shape(laptop(false))
        "imac" -> Shape(
            listOf(Part(rr(2.5f, 3f, 19f, 13.5f, 1.8f)), Part("M2.5 13.2H21.5M10.2 16.5L9.7 20H14.3L13.8 16.5M8.5 20H15.5")),
            Mark("apple", 12f, 8.2f),
        )
        "mac_desktop" -> Shape(listOf(Part(rr(3f, 8f, 18f, 8f, 2.4f)), Part("M6 18.5H18")), Mark("apple", 12f, 12f, 0.85f))
        "win_desktop" -> Shape(monitor, Mark("windows", 12f, 10f))
        "linux_desktop" -> Shape(listOf(Part(rr(6.5f, 2f, 11f, 20f, 1.5f)), Part("M9.5 5.5H14.5M9.5 8H14.5")), Mark("tux", 12f, 14.5f))
        "desktop" -> Shape(monitor)
        "tv" -> Shape(listOf(Part(rr(2f, 7f, 20f, 13f, 2f)), Part("M8 3L12 7L16 3")))
        "watch" -> Shape(listOf(Part(rr(6f, 6f, 12f, 12f, 3f)), Part("M9 6L10 2H14L15 6M9 18L10 22H14L15 18")))
        else -> Shape(
            listOf(
                Part(rr(4f, 4f, 16f, 16f, 3f)),
                Part("M9.8 9.8A2.3 2.3 0 1 1 13 11.9C12.4 12.2 12 12.7 12 13.3V13.6"),
                Part(circle(12f, 16.3f, 0.4f), fill = true),
            ),
        )
    }

    /** A phone, tablet, foldable or watch icon (a phone's page layout). */
    fun isHandheld(icon: String?): Boolean =
        icon in setOf("phone", "tablet", "watch", "android_phone", "android_tablet", "android_foldable")
}
