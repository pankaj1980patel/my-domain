package com.mydomain.android.core

/**
 * Which id this phone goes by (the pure part, unit-tested; [com.mydomain.android.DeviceId]
 * reads the phone and keeps the answer).
 *
 * Your other devices know this phone by that id, so it must never change
 * under an install that already has one:
 *  1. An id kept from an earlier launch wins, always.
 *  2. Else the id earlier versions derived: the IMEI, which they could read
 *     only on Android 9 or older and only where READ_PHONE_STATE had been
 *     granted (this version never asks for it), else ANDROID_ID. On an
 *     install made by this version the IMEI is never readable, so this is
 *     ANDROID_ID: what a new install uses anyway.
 *  3. Else (no ANDROID_ID: earlier versions then had the core make up a
 *     new id on every launch, so there's nothing to keep) a random id,
 *     kept from then on.
 * Whatever comes out is kept, so the next launch takes step 1.
 */
object DeviceIdPlan {
    fun choose(stored: String?, legacyImei: String?, androidId: String?, newRandom: () -> String): String {
        stored?.trim()?.takeIf { it.isNotEmpty() }?.let { return it }
        legacyImei?.trim()?.takeIf { it.isNotEmpty() }?.let { return it }
        androidId?.trim()?.takeIf { it.isNotEmpty() }?.let { return it }
        return newRandom()
    }
}
