package com.mydomain.android

import android.os.Build

/**
 * This app targets Android 9 (SDK 28) on purpose (see build.gradle.kts):
 * a newer Android's rules apply only to apps that target it. So "does the
 * Android 12 Bluetooth permission apply?" depends on the phone AND the
 * target, which [rules] answers.
 */
object Sdk {
    /** Android [level]'s rules apply to this app: the phone runs it and the
     *  app targets it. */
    fun rules(level: Int) = Build.VERSION.SDK_INT >= level && BuildConfig.TARGET_SDK >= level
}
