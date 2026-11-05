package com.mydomain.android.core

/**
 * How a device's screen is arranged ("Arrange"): which tiles come first and
 * which are hidden. Like the desktop's `page_layout.rs`, with the same rules:
 *
 *  - one layout per kind of device ("phone" / "computer"), and optionally
 *    one for a single device ("Only for this device", by node id) that wins
 *    over its kind's;
 *  - a layout lists tile ids; the screen passes the tiles it has now and
 *    gets them back in the saved order ([merge]): a tile the layout doesn't
 *    know (added in a later version) appears at its default place, and ids
 *    the screen no longer has are ignored, so nothing is ever lost.
 *
 * Pure (no Android): saved by [PageLayoutStore] in the "md" preferences.
 */
data class PageLayout(val order: List<String> = emptyList(), val hidden: List<String> = emptyList())

/** Where the layout a screen shows came from. */
enum class LayoutScope { DEFAULT, KIND, DEVICE }

/** What a device's screen shows: every tile passed, in order, the hidden
 *  ones, and where that came from. */
data class PageLayoutView(val order: List<String>, val hidden: List<String>, val scope: LayoutScope) {
    /** The tiles shown, in order. */
    val shown: List<String> get() = order.filter { it !in hidden }
}

/** Every saved layout: per kind, and per device (node id). */
data class SavedLayouts(
    val kinds: Map<String, PageLayout> = emptyMap(),
    val devices: Map<String, PageLayout> = emptyMap(),
)

object PageArrangement {
    const val MAX_TILES = 100
    const val MAX_ID = 64
    const val MAX_DEVICES = 200

    /** "phone" stays "phone"; anything else is a computer. */
    fun kindOf(kind: String): String = if (kind == "phone") "phone" else "computer"

    /** [layout] made safe to keep: blank, overlong and repeated ids dropped,
     *  at most [MAX_TILES] of each list. */
    fun clean(layout: PageLayout): PageLayout {
        fun tidy(ids: List<String>) = ids.filter { it.isNotBlank() && it.length <= MAX_ID }.distinct().take(MAX_TILES)
        return PageLayout(tidy(layout.order), tidy(layout.hidden))
    }

    /** [tiles] (the screen's tiles, in default order) arranged by [saved]:
     *  the saved order first (ids not in [tiles] skipped), each tile [saved]
     *  doesn't list placed right after the tile that comes before it by
     *  default (first if none does). Hidden: those of [saved] that are tiles. */
    fun merge(saved: PageLayout, tiles: List<String>): PageLayout {
        val known = tiles.toSet()
        val order = saved.order.filter { it in known }.distinct().toMutableList()
        tiles.forEachIndexed { i, id ->
            if (id in order) return@forEachIndexed
            // The nearest tile before it by default that's already placed.
            var at = 0
            for (j in i - 1 downTo 0) {
                val p = order.indexOf(tiles[j])
                if (p >= 0) {
                    at = p + 1
                    break
                }
            }
            order.add(at, id)
        }
        return PageLayout(order, saved.hidden.filter { it in known }.distinct())
    }

    /** [order] with [id] moved onto [over]'s place. */
    fun moveOnto(order: List<String>, id: String, over: String): List<String> {
        val from = order.indexOf(id)
        val to = order.indexOf(over)
        if (from < 0 || to < 0 || from == to) return order
        val out = order.filter { it != id }.toMutableList()
        // Moving down: after [over]; moving up: before it.
        out.add(out.indexOf(over) + if (from < to) 1 else 0, id)
        return out
    }

    /** [order] with [id] one shown tile earlier ([step] -1) or later (+1);
     *  hidden tiles are stepped over. Unchanged at either end. */
    fun moveStep(order: List<String>, hidden: List<String>, id: String, step: Int): List<String> {
        val shown = order.filter { it == id || it !in hidden }
        val i = shown.indexOf(id)
        val next = shown.getOrNull(i + step)
        if (i < 0 || next == null) return order
        return moveOnto(order, id, next)
    }

    /** What [node]'s screen (a device of [kind]) shows, given its [tiles]
     *  in default order. */
    fun view(saved: SavedLayouts, kind: String, node: String, tiles: List<String>): PageLayoutView {
        val (layout, scope) = saved.devices[node]?.let { it to LayoutScope.DEVICE }
            ?: saved.kinds[kindOf(kind)]?.let { it to LayoutScope.KIND }
            ?: (PageLayout() to LayoutScope.DEFAULT)
        val merged = merge(layout, tiles)
        return PageLayoutView(merged.order, merged.hidden, scope)
    }

    /** Keep [layout] for [node] alone ([onlyThis]) or for every device of
     *  [kind] (then [node] follows it too: its own layout goes). Null
     *  resets: with [onlyThis], [node] follows its kind's layout again;
     *  otherwise the kind's layout and [node]'s own both go (the default). */
    fun apply(saved: SavedLayouts, kind: String, node: String, onlyThis: Boolean, layout: PageLayout?): SavedLayouts {
        val l = layout?.let(::clean)
        val k = kindOf(kind)
        return if (onlyThis) {
            when {
                l == null -> saved.copy(devices = saved.devices - node)
                node !in saved.devices && saved.devices.size >= MAX_DEVICES -> saved
                else -> saved.copy(devices = saved.devices + (node to l))
            }
        } else {
            SavedLayouts(
                kinds = if (l == null) saved.kinds - k else saved.kinds + (k to l),
                devices = saved.devices - node,
            )
        }
    }
}
