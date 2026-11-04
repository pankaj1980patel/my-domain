package com.mydomain.android

import android.Manifest
import android.app.NotificationManager
import android.appwidget.AppWidgetManager
import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.net.Uri
import android.os.Build
import android.os.PowerManager
import android.provider.Settings
import androidx.core.app.NotificationManagerCompat
import androidx.core.content.ContextCompat
import com.mydomain.android.core.SetupChecklist
import com.mydomain.android.core.SetupItem
import com.mydomain.android.core.SetupRow

/**
 * "Set up this phone" on Android: reads whether each [SetupItem] is on and
 * opens the screen where the user turns it on (Android's Settings page, or
 * Android's own permission prompt, asked by the UI with [permissions]).
 * Nothing here turns anything on by itself.
 */
object SetupState {
    /** The home screen's card was dismissed (or opened): never shown again. */
    const val PREF_CARD_DISMISSED = "setup_card_dismissed"

    fun checklist() = SetupChecklist(Build.VERSION.SDK_INT, BuildConfig.TARGET_SDK)

    fun rows(ctx: Context): List<SetupRow> = checklist().rows { isOn(ctx, it) }

    fun isOn(ctx: Context, item: SetupItem): Boolean = runCatching {
        val granted = { p: String -> ContextCompat.checkSelfPermission(ctx, p) == PackageManager.PERMISSION_GRANTED }
        when (item) {
            SetupItem.NOTIFICATIONS -> NotificationManagerCompat.from(ctx).areNotificationsEnabled()
            SetupItem.NOTIFICATION_ACCESS ->
                ctx.packageName in NotificationManagerCompat.getEnabledListenerPackages(ctx)
            SetupItem.ACCESSIBILITY_HELPER -> PhoneHelper.enabled(ctx)
            SetupItem.BATTERY ->
                ctx.getSystemService(PowerManager::class.java)?.isIgnoringBatteryOptimizations(ctx.packageName) == true
            SetupItem.NEARBY_DEVICES -> Build.VERSION.SDK_INT < Build.VERSION_CODES.S ||
                (granted(Manifest.permission.BLUETOOTH_ADVERTISE) && granted(Manifest.permission.BLUETOOTH_CONNECT))
            SetupItem.LOCATION_SHARING -> PhoneLocation.allowed(ctx)
            SetupItem.LOCATION_PERMISSION -> granted(Manifest.permission.ACCESS_FINE_LOCATION)
            SetupItem.PHOTOS -> Gallery.canRead(ctx)
            SetupItem.MICROPHONE -> granted(Manifest.permission.RECORD_AUDIO)
            SetupItem.WRITE_SETTINGS -> ScreenSettings.canWrite(ctx)
            SetupItem.FULL_SCREEN -> Build.VERSION.SDK_INT < Build.VERSION_CODES.UPSIDE_DOWN_CAKE ||
                ctx.getSystemService(NotificationManager::class.java)?.canUseFullScreenIntent() == true
            SetupItem.SHIZUKU -> Privileged.state() == Privileged.State.READY
            SetupItem.WIDGET -> AppWidgetManager.getInstance(ctx)
                .getAppWidgetIds(ComponentName(ctx, HomeWidget::class.java)).isNotEmpty()
            SetupItem.QUICK_TILES -> false
        }
    }.getOrDefault(false)

    /** Android's permission prompt for [item], when that's how it's turned
     *  on here; null when it's a Settings screen instead ([open]). */
    fun permissions(item: SetupItem): Array<String>? = when (item) {
        SetupItem.NOTIFICATIONS ->
            if (Sdk.rules(Build.VERSION_CODES.TIRAMISU)) arrayOf(Manifest.permission.POST_NOTIFICATIONS) else null
        SetupItem.NEARBY_DEVICES ->
            if (Sdk.rules(Build.VERSION_CODES.S)) {
                arrayOf(Manifest.permission.BLUETOOTH_ADVERTISE, Manifest.permission.BLUETOOTH_CONNECT)
            } else null
        SetupItem.LOCATION_PERMISSION -> arrayOf(Manifest.permission.ACCESS_FINE_LOCATION)
        SetupItem.PHOTOS -> Gallery.permissions()
        SetupItem.MICROPHONE -> arrayOf(Manifest.permission.RECORD_AUDIO)
        else -> null
    }

    /** After Android's prompt for [item] was answered. */
    fun onAnswered(ctx: Context, item: SetupItem) {
        val app = ctx.applicationContext
        when (item) {
            SetupItem.PHOTOS -> {
                FileActions.shareFolders(app)
                PhotoBackup.scanSoon()
            }
            SetupItem.NEARBY_DEVICES -> BleLink.start(app)
            SetupItem.LOCATION_PERMISSION -> Controls.onChange()
            else -> {}
        }
    }

    /** Open the Settings screen where the user turns [item] on: the exact
     *  page when this Android has one, else the nearest. False: none opened. */
    fun open(ctx: Context, item: SetupItem): Boolean {
        if (item == SetupItem.SHIZUKU) return openShizuku(ctx)
        if (item == SetupItem.WIDGET) return pinWidget(ctx)
        return intents(ctx, item).any { runCatching { ctx.startActivity(it) }.isSuccess }
    }

    /** This app's page in Android's Settings (permissions), for a
     *  permission Android no longer prompts for ("Don't allow" twice). */
    fun openAppDetails(ctx: Context): Boolean =
        runCatching { ctx.startActivity(appDetails(ctx)) }.isSuccess

    private fun appDetails(ctx: Context) =
        Intent(Settings.ACTION_APPLICATION_DETAILS_SETTINGS, Uri.parse("package:${ctx.packageName}"))

    private fun intents(ctx: Context, item: SetupItem): List<Intent> {
        val pkg = Uri.parse("package:${ctx.packageName}")
        val appNotifications = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            Intent(Settings.ACTION_APP_NOTIFICATION_SETTINGS).putExtra(Settings.EXTRA_APP_PACKAGE, ctx.packageName)
        } else null
        return when (item) {
            SetupItem.NOTIFICATIONS -> listOfNotNull(appNotifications, appDetails(ctx))
            SetupItem.NOTIFICATION_ACCESS -> listOfNotNull(
                if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
                    Intent(Settings.ACTION_NOTIFICATION_LISTENER_DETAIL_SETTINGS).putExtra(
                        Settings.EXTRA_NOTIFICATION_LISTENER_COMPONENT_NAME,
                        ComponentName(ctx, NotifListener::class.java).flattenToString(),
                    )
                } else null,
                Intent(Settings.ACTION_NOTIFICATION_LISTENER_SETTINGS),
            )
            SetupItem.ACCESSIBILITY_HELPER -> listOf(Intent(Settings.ACTION_ACCESSIBILITY_SETTINGS))
            // The list of apps, not the one-tap "Allow?" dialog: that needs
            // REQUEST_IGNORE_BATTERY_OPTIMIZATIONS, which isn't declared.
            SetupItem.BATTERY -> listOf(Intent(Settings.ACTION_IGNORE_BATTERY_OPTIMIZATION_SETTINGS), appDetails(ctx))
            SetupItem.WRITE_SETTINGS -> listOf(Intent(Settings.ACTION_MANAGE_WRITE_SETTINGS, pkg), appDetails(ctx))
            SetupItem.FULL_SCREEN -> listOfNotNull(
                if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.UPSIDE_DOWN_CAKE) {
                    Intent(Settings.ACTION_MANAGE_APP_USE_FULL_SCREEN_INTENT, pkg)
                } else null,
                appNotifications,
                appDetails(ctx),
            )
            SetupItem.NEARBY_DEVICES, SetupItem.LOCATION_PERMISSION, SetupItem.PHOTOS, SetupItem.MICROPHONE -> listOf(appDetails(ctx))
            SetupItem.LOCATION_SHARING, SetupItem.SHIZUKU, SetupItem.WIDGET, SetupItem.QUICK_TILES -> emptyList()
        }
    }

    /** Shizuku: get it, start it, or answer its own permission prompt. */
    fun openShizuku(ctx: Context): Boolean = when (Privileged.state()) {
        Privileged.State.NOT_INSTALLED -> {
            val market = Intent(Intent.ACTION_VIEW, Uri.parse("market://details?id=${Privileged.SHIZUKU_PACKAGE}"))
            val web = Intent(Intent.ACTION_VIEW, Uri.parse("https://shizuku.rikka.app/download/"))
            listOf(market, web).any { runCatching { ctx.startActivity(it) }.isSuccess }
        }
        Privileged.State.NOT_RUNNING -> ctx.packageManager.getLaunchIntentForPackage(Privileged.SHIZUKU_PACKAGE)
            ?.let { runCatching { ctx.startActivity(it) }.isSuccess } ?: false
        Privileged.State.NOT_ALLOWED -> { Privileged.ask(); true }
        Privileged.State.READY -> true
    }

    /** The launcher can add the widget after its own confirmation (Android 8+). */
    fun canPinWidget(ctx: Context): Boolean =
        Build.VERSION.SDK_INT >= Build.VERSION_CODES.O &&
            runCatching { AppWidgetManager.getInstance(ctx).isRequestPinAppWidgetSupported }.getOrDefault(false)

    /** Ask the launcher to add the widget; the launcher asks the user. */
    private fun pinWidget(ctx: Context): Boolean {
        if (!canPinWidget(ctx) || Build.VERSION.SDK_INT < Build.VERSION_CODES.O) return false
        return runCatching {
            AppWidgetManager.getInstance(ctx).requestPinAppWidget(ComponentName(ctx, HomeWidget::class.java), null, null)
        }.getOrDefault(false)
    }
}
