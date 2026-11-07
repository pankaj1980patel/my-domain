package com.mydomain.android.core

import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.util.Base64
import androidx.compose.runtime.mutableStateMapOf
import com.mydomain.android.ffi.MdEngine
import org.json.JSONObject

/**
 * The wallpaper look each of your devices shared (the core's
 * `features::wallpaper`): up to four colours and, when that device could
 * read it, a small picture. The device art ([com.mydomain.android.ui.DeviceArt])
 * puts it on the drawn device's screen. Loaded once ([load]) and kept up
 * to date by the `wallpaper` event ([apply]). This phone's own look is sent
 * by [com.mydomain.android.PhoneWallpaper].
 */
object Wallpapers {
    /** One device's look: [colors] as ARGB ints (the most seen first),
     *  [image] the decoded picture, if it sent one. */
    class Look(val hash: String, val colors: List<Int>, val image: Bitmap?)

    /** node id -> its look (changed on the main thread). */
    val looks = mutableStateMapOf<String, Look>()

    private fun color(s: String): Int? {
        val t = s.trim()
        if (!Regex("^#[0-9a-fA-F]{6}$").matches(t)) return null
        return (0xFF000000L or t.substring(1).toLong(16)).toInt()
    }

    /** A `data:image/...;base64,` URL as a bitmap (null: not one, or broken). */
    private fun decode(url: String?): Bitmap? {
        if (url.isNullOrEmpty() || !url.startsWith("data:image/")) return null
        val comma = url.indexOf(',').takeIf { it > 0 } ?: return null
        return runCatching {
            val bytes = Base64.decode(url.substring(comma + 1), Base64.DEFAULT)
            BitmapFactory.decodeByteArray(bytes, 0, bytes.size)
        }.getOrNull()
    }

    fun parse(o: JSONObject?): Look? {
        o ?: return null
        val arr = o.optJSONArray("colors")
        val colors = (0 until (arr?.length() ?: 0)).mapNotNull { color(arr!!.optString(it)) }
        val image = if (o.isNull("image")) null else decode(o.optString("image"))
        if (colors.isEmpty() && image == null) return null
        return Look(o.optString("hash"), colors, image)
    }

    /** Every look the core keeps (after start). Any thread: the pictures
     *  are decoded here, off the main thread when called from one. */
    fun load(engine: MdEngine) {
        val json = runCatching { JSONObject(engine.wallpapersJson()) }.getOrNull() ?: return
        val all = HashMap<String, Look>()
        for (node in json.keys()) parse(json.optJSONObject(node))?.let { all[node] = it }
        Core.onMain {
            looks.clear()
            looks.putAll(all)
        }
    }

    /** A `wallpaper` event: `{node, wallpaper}` (null: it stopped sharing).
     *  The picture is decoded on the caller's thread. */
    fun parseEvent(o: JSONObject): Pair<String, Look?>? {
        val node = o.optString("node").ifEmpty { return null }
        val w = if (o.isNull("wallpaper")) null else o.optJSONObject("wallpaper")
        return node to parse(w)
    }

    /** Main thread. */
    fun set(node: String, look: Look?) {
        if (look == null) looks.remove(node) else looks[node] = look
    }

    /** Sample mode: made-up colours for some devices, none for others (so
     *  the built-in art shows too). */
    fun sample(byNode: Map<String, List<String>>) {
        looks.clear()
        byNode.forEach { (node, cs) ->
            val colors = cs.mapNotNull { color(it) }
            if (colors.isNotEmpty()) looks[node] = Look("sample", colors, null)
        }
    }
}
