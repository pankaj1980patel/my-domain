package com.mydomain.android.core

import android.content.Context
import androidx.compose.runtime.mutableStateMapOf
import com.mydomain.android.ffi.MdEngine
import org.json.JSONArray
import org.json.JSONObject

/** What you call one of your devices on this phone: a nickname (null: its
 *  own name), an icon from [DeviceLabels.ICONS] and an accent colour from
 *  [DeviceLabels.COLORS] (null: none). */
data class DeviceLabel(val nickname: String? = null, val icon: String? = null, val color: String? = null) {
    val isEmpty: Boolean get() = nickname == null && icon == null && color == null
}

/**
 * Device labels: a nickname, an icon and a colour you give each of your
 * devices. Like the desktop's `device_labels.rs`, with the same icon and
 * colour sets. Kept on this phone, and shared with your other devices while
 * "Share names and icons with my other devices" is on (the core's
 * `features::labels`, on until turned off): [Core.setDeviceLabel] hands a
 * label to the core, and labels set on your other devices arrive as the
 * `labels_changed` event ([applyShared]), which changes them here without
 * sending anything back.
 *
 * In the "md" SharedPreferences, key [PREF]: a JSON object node id ->
 * `{nickname?, icon?, color?}`. [Core.peerName] prefers the nickname, so
 * screens, tiles, the widget and notifications all show it.
 */
object DeviceLabels {
    const val PREF = "device_labels"
    const val MAX_NICKNAME = 40
    val ICONS = listOf("phone", "tablet", "laptop", "desktop", "tv", "watch")
    val COLORS = listOf("cyan", "blue", "sky", "green", "gold", "rose", "grey")

    /** node id -> its label (changed on the main thread). */
    val labels = mutableStateMapOf<String, DeviceLabel>()

    private var loaded = false

    private fun prefs(ctx: Context) = ctx.getSharedPreferences("md", Context.MODE_PRIVATE)

    /** Read the saved labels, once (from [Core.init]). */
    @Synchronized
    fun load(ctx: Context) {
        if (loaded) return
        loaded = true
        val saved = decode(prefs(ctx.applicationContext).getString(PREF, null))
        labels.clear()
        labels.putAll(saved)
    }

    fun nickname(node: String?): String? = node?.let { labels[it]?.nickname }

    /** Set [node]'s label (all blank: forget it) and save; returns it as
     *  set. Main thread. */
    fun set(ctx: Context, node: String, nickname: String?, icon: String?, color: String?): DeviceLabel {
        val label = checked(nickname, icon, color)
        if (label.isEmpty) labels.remove(node) else labels[node] = label
        save(ctx)
        return label
    }

    private fun save(ctx: Context) {
        prefs(ctx.applicationContext).edit().putString(PREF, encode(labels.toMap())).apply()
    }

    /** Labels set on your other devices (the `labels` array of a
     *  `labels_changed` event): change them here (a cleared one forgets the
     *  label) and save. Nothing goes back to the core. Main thread. */
    fun applyShared(ctx: Context, arr: JSONArray?) {
        if (arr == null) return
        for (i in 0 until arr.length()) {
            val o = arr.optJSONObject(i) ?: continue
            val node = o.optString("node").ifEmpty { null } ?: continue
            val label = checked(o.text("nickname"), o.text("icon"), o.text("color"))
            if (label.isEmpty) labels.remove(node) else labels[node] = label
        }
        save(ctx)
    }

    /** Hand the labels this phone had before labels were shared to the core
     *  (at start): ones it doesn't know yet, which anything set since wins
     *  over. */
    fun seed(engine: MdEngine) {
        for ((node, l) in labels.toMap()) {
            runCatching { engine.seedLabel(node, l.nickname, l.icon, l.color) }
        }
    }

    /** A string field, or null when it's missing, null or blank. */
    private fun JSONObject.text(key: String): String? =
        if (isNull(key)) null else optString(key).ifEmpty { null }

    /** The nickname tidied (spaces collapsed, cut to [MAX_NICKNAME]; blank:
     *  none); an icon or colour not in its set is dropped. */
    fun checked(nickname: String?, icon: String?, color: String?): DeviceLabel = DeviceLabel(
        nickname = nickname?.trim()?.split(Regex("\\s+"))?.joinToString(" ")?.take(MAX_NICKNAME)?.trim()?.ifEmpty { null },
        icon = icon?.takeIf { it in ICONS },
        color = color?.takeIf { it in COLORS },
    )

    fun encode(map: Map<String, DeviceLabel>): String {
        val o = JSONObject()
        for ((node, l) in map) {
            if (l.isEmpty) continue
            val j = JSONObject()
            l.nickname?.let { j.put("nickname", it) }
            l.icon?.let { j.put("icon", it) }
            l.color?.let { j.put("color", it) }
            o.put(node, j)
        }
        return o.toString()
    }

    fun decode(json: String?): Map<String, DeviceLabel> {
        if (json.isNullOrBlank()) return emptyMap()
        val o = runCatching { JSONObject(json) }.getOrNull() ?: return emptyMap()
        val out = mutableMapOf<String, DeviceLabel>()
        for (node in o.keys()) {
            val j = o.optJSONObject(node) ?: continue
            val l = checked(
                j.optString("nickname").ifEmpty { null },
                j.optString("icon").ifEmpty { null },
                j.optString("color").ifEmpty { null },
            )
            if (!l.isEmpty) out[node] = l
        }
        return out
    }
}
