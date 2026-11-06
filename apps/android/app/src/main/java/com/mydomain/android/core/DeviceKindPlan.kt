package com.mydomain.android.core

/** What kind of device one is (the core's `DeviceKind`): its OS, form
 *  (phone, tablet, foldable, laptop, desktop, all_in_one, tv), model and
 *  maker for display, and the icon the core worked out. */
data class DeviceKind(
    val os: String = "",
    val form: String = "",
    val model: String = "",
    val maker: String = "",
    val icon: String = "unknown",
) {
    /** "Samsung Galaxy S23", "Apple MacBook Pro", "Pixel 8"; "" when it
     *  never said. */
    val description: String get() = DeviceKindPlan.describe(maker, model)
}

/**
 * The pure rules for this phone's own kind and for showing others' (tested
 * on the JVM): the form from the screen and the system features, the model
 * worth showing, the maker tidied.
 */
object DeviceKindPlan {
    /** At least this smallest width (dp) is a tablet, as Android's own
     *  `sw600dp` resources say. */
    const val TABLET_DP = 600

    /** This device's form: a TV (leanback) first, then a foldable (it has
     *  a hinge sensor), then a tablet by its smallest width, else a phone. */
    fun form(smallestWidthDp: Int, hinge: Boolean, leanback: Boolean): String = when {
        leanback -> "tv"
        hinge -> "foldable"
        smallestWidthDp >= TABLET_DP -> "tablet"
        else -> "phone"
    }

    private val CODE = Regex("^[A-Z]{0,4}-?[A-Z]?[0-9][A-Z0-9-]{2,}$")

    /** A model that's a maker's code ("I2405", "SM-S918B", "CPH2451",
     *  "23049PCD8G", "V2318"), not a name people know. */
    fun looksLikeCode(model: String): Boolean = CODE.matches(model.trim())

    /**
     * The model to show: Android's model, unless it's a code and the
     * phone's device name (Settings.Global DEVICE_NAME, which most makers
     * set to the marketing name, e.g. "vivo T3 Ultra") is a name rather
     * than something the user typed ("Pankaj's phone").
     */
    fun displayModel(maker: String, model: String, deviceName: String?): String {
        val m = tidy(model)
        val name = tidy(deviceName.orEmpty())
        if (m.isNotEmpty() && !looksLikeCode(m)) return m
        val personal = name.contains('\'') || name.contains('’') || name.contains(" de ") || name.lowercase().contains(" phone")
        if (name.isNotEmpty() && !personal && !name.equals(m, ignoreCase = true)) {
            // "vivo T3 Ultra" with maker "vivo": the model part.
            return if (maker.isNotEmpty() && name.startsWith(maker, ignoreCase = true) && name.length > maker.length) {
                name.substring(maker.length).trim()
            } else {
                name
            }
        }
        return m
    }

    /** Build.MANUFACTURER for display: "samsung" -> "Samsung", "HUAWEI" ->
     *  "Huawei", known spellings kept ("iQOO", "OnePlus", "HMD"). */
    fun tidyMaker(maker: String): String {
        val m = tidy(maker)
        val known = mapOf(
            "iqoo" to "iQOO", "oneplus" to "OnePlus", "hmd global" to "HMD", "hmd" to "HMD",
            "lge" to "LG", "lg" to "LG", "zte" to "ZTE", "tcl" to "TCL", "htc" to "HTC", "realme" to "realme",
        )
        known[m.lowercase()]?.let { return it }
        val shouting = m.any { it.isLetter() } && (m == m.lowercase() || m == m.uppercase())
        if (!shouting) return m
        return m.split(' ').joinToString(" ") { w -> w.lowercase().replaceFirstChar { it.titlecase() } }
    }

    /** Maker and model together, without saying the maker twice. */
    fun describe(maker: String, model: String): String {
        val mk = maker.trim()
        val md = model.trim()
        if (md.isEmpty()) return mk
        if (mk.isEmpty() || md.startsWith(mk, ignoreCase = true)) return md
        return "$mk $md"
    }

    /** This phone's own icon from its form (the core's `icon_for` for
     *  Android). */
    fun androidIcon(form: String): String = when (form) {
        "tablet" -> "android_tablet"
        "foldable" -> "android_foldable"
        "tv" -> "tv"
        else -> "android_phone"
    }

    /** The icon drawn for a device: the one you chose (label), else what
     *  the core worked out from what it said, else "unknown". */
    fun iconOf(labelIcon: String?, kind: DeviceKind?): String =
        labelIcon?.takeIf { it in DeviceIconShapes.ICONS }
            ?: kind?.icon?.takeIf { it in DeviceIconShapes.ICONS }
            ?: "unknown"

    /** Spaces collapsed, control characters dropped, at most 60 characters. */
    fun tidy(s: String): String =
        s.filterNot { it.isISOControl() }.split(Regex("\\s+")).filter { it.isNotEmpty() }.joinToString(" ").take(60).trim()
}
