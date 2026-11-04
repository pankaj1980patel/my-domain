package com.mydomain.android.core

/**
 * "Set up this phone": the optional permissions and settings each feature
 * needs. Pure (no Android types), so which items apply on which Android
 * version, their order and the "N of M on" count are unit-tested. The
 * Android side ([com.mydomain.android.SetupState]) reads each item's state
 * and opens the screen that changes it; the app never turns anything on
 * itself.
 */
enum class SetupItem(
    /** The phone must run at least this Android version (API level). */
    val minSdk: Int = 1,
    /** A runtime permission introduced at [minSdk]: it also needs the app to
     *  target that version (an older target never asks; see Sdk.kt). */
    val needsTarget: Boolean = false,
    /** Shown with an explanation only: its state can't be read. */
    val infoOnly: Boolean = false,
) {
    NOTIFICATIONS,
    NOTIFICATION_ACCESS,
    ACCESSIBILITY_HELPER,
    BATTERY,
    NEARBY_DEVICES(minSdk = 31, needsTarget = true),
    LOCATION_SHARING,
    LOCATION_PERMISSION,
    PHOTOS,
    MICROPHONE(minSdk = 29),
    WRITE_SETTINGS,
    FULL_SCREEN(minSdk = 34),
    SHIZUKU,
    WIDGET,
    QUICK_TILES(infoOnly = true),
}

enum class SetupStatus { ON, OFF, NOT_NEEDED, INFO }

data class SetupRow(val item: SetupItem, val status: SetupStatus)

class SetupChecklist(private val sdkInt: Int, private val targetSdk: Int) {

    /** [item] matters on this phone (Android version and app target). */
    fun applies(item: SetupItem): Boolean =
        sdkInt >= item.minSdk && (!item.needsTarget || targetSdk >= item.minSdk)

    /** Every item, in display order: the ones that apply first (in the
     *  enum's order, which never changes as you turn things on), then the
     *  explain-only ones, then those not needed on this Android version.
     *  [isOn] is asked only for items that apply and can be read. */
    fun rows(isOn: (SetupItem) -> Boolean): List<SetupRow> {
        val all = SetupItem.entries.map { item ->
            val status = when {
                !applies(item) -> SetupStatus.NOT_NEEDED
                item.infoOnly -> SetupStatus.INFO
                isOn(item) -> SetupStatus.ON
                else -> SetupStatus.OFF
            }
            SetupRow(item, status)
        }
        return all.sortedBy { rank(it.status) }
    }

    private fun rank(s: SetupStatus) = when (s) {
        SetupStatus.ON, SetupStatus.OFF -> 0
        SetupStatus.INFO -> 1
        SetupStatus.NOT_NEEDED -> 2
    }

    companion object {
        /** How many are on, of those that can be on here. */
        fun count(rows: List<SetupRow>): Pair<Int, Int> {
            val counted = rows.filter { it.status == SetupStatus.ON || it.status == SetupStatus.OFF }
            return counted.count { it.status == SetupStatus.ON } to counted.size
        }

        /** The home screen's one-time "Set up this phone" card: signed in,
         *  never dismissed (or opened), and something still off. */
        fun showCard(signedIn: Boolean, dismissed: Boolean, on: Int, total: Int): Boolean =
            signedIn && !dismissed && on < total
    }
}
