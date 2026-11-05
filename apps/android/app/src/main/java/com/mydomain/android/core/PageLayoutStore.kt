package com.mydomain.android.core

import android.content.Context
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import org.json.JSONArray
import org.json.JSONObject

/**
 * The saved device-screen layouts ([PageArrangement] has the rules), in the
 * "md" SharedPreferences under [PREF] as JSON:
 * `{"kinds":{"phone":{"order":[…],"hidden":[…]}},"devices":{"<node>":{…}}}`.
 * Local to this phone; never synced.
 */
object PageLayoutStore {
    const val PREF = "page_layout"

    /** Every saved layout (read once; changed on the main thread). */
    var saved by mutableStateOf(SavedLayouts()); private set
    private var loaded = false

    private fun prefs(ctx: Context) = ctx.applicationContext.getSharedPreferences("md", Context.MODE_PRIVATE)

    /** Read the saved layouts, once. */
    fun load(ctx: Context) {
        if (loaded) return
        loaded = true
        saved = runCatching { decode(prefs(ctx).getString(PREF, null)) }.getOrDefault(SavedLayouts())
    }

    /** What [node]'s screen shows (see [PageArrangement.view]). */
    fun view(kind: String, node: String, tiles: List<String>): PageLayoutView =
        PageArrangement.view(saved, kind, node, tiles)

    /** Save (see [PageArrangement.apply]): shown at once, written in the
     *  background (`apply`). */
    fun set(ctx: Context, kind: String, node: String, onlyThis: Boolean, layout: PageLayout?) {
        load(ctx)
        saved = PageArrangement.apply(saved, kind, node, onlyThis, layout)
        prefs(ctx).edit().putString(PREF, encode(saved).toString()).apply()
    }

    private fun decode(json: String?): SavedLayouts {
        if (json.isNullOrBlank()) return SavedLayouts()
        val o = JSONObject(json)
        fun list(a: JSONArray?) = (0 until (a?.length() ?: 0)).mapNotNull { a?.optString(it)?.takeIf(String::isNotEmpty) }
        fun map(m: JSONObject?): Map<String, PageLayout> {
            if (m == null) return emptyMap()
            val out = LinkedHashMap<String, PageLayout>()
            for (key in m.keys()) {
                val l = m.optJSONObject(key) ?: continue
                out[key] = PageArrangement.clean(PageLayout(list(l.optJSONArray("order")), list(l.optJSONArray("hidden"))))
            }
            return out
        }
        return SavedLayouts(map(o.optJSONObject("kinds")), map(o.optJSONObject("devices")))
    }

    private fun encode(s: SavedLayouts): JSONObject {
        fun map(m: Map<String, PageLayout>) = JSONObject().apply {
            for ((k, l) in m) put(k, JSONObject().put("order", JSONArray(l.order)).put("hidden", JSONArray(l.hidden)))
        }
        return JSONObject().put("kinds", map(s.kinds)).put("devices", map(s.devices))
    }
}
