package com.mydomain.android

import android.app.Application

/**
 * The app's process: the first thing that runs, so the crash log
 * ([CrashLog]) catches a crash anywhere after it, the service and the
 * screens included. Everything else still starts where it did
 * (MainActivity, NetService).
 */
class MdApp : Application() {
    override fun onCreate() {
        super.onCreate()
        CrashLog.install(this)
    }
}
