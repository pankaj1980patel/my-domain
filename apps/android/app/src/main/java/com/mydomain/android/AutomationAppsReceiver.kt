package com.mydomain.android

import android.app.Activity
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.os.Build
import android.util.Base64
import android.util.Log
import androidx.compose.runtime.mutableStateListOf
import androidx.core.app.NotificationCompat
import com.mydomain.android.core.AppPeer
import com.mydomain.android.core.AppRefusal
import com.mydomain.android.core.AppRequest
import com.mydomain.android.core.AppRun
import com.mydomain.android.core.AutomationApps
import com.mydomain.android.core.AutomationBook
import com.mydomain.android.core.Core
import com.mydomain.android.core.DeviceLabels
import com.mydomain.android.core.KeystoreSecrets
import com.mydomain.android.core.RateWindow
import com.mydomain.android.core.friendlyError
import com.mydomain.android.core.niceName
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.async
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import kotlinx.coroutines.withTimeoutOrNull
import org.json.JSONObject
import java.security.SecureRandom
import java.util.concurrent.atomic.AtomicInteger

/**
 * Automation apps on this phone (Tasker, MacroDroid, Automate, ...) asking
 * my-domain to do what its own buttons do: a device action, text to a
 * device, one of this phone's automations, or the list of your devices.
 * The phone's counterpart of the desktop's webhooks. Set up in Settings
 * (Automation apps).
 *
 * What keeps it safe:
 *  - off until switched on in Settings; while off, every request is refused;
 *  - two ways in, both explicit (no intent filters: an app names the
 *    receiver's class, and nothing else on the phone sees the request):
 *    [AutomationAppsReceiver] only takes broadcasts from apps holding
 *    [AutomationApps.PERMISSION], a "dangerous" permission Android asks you
 *    about for each app that wants it; [AutomationAppsKeyReceiver] (for
 *    Tasker and the like, which can't ask for another app's permission)
 *    needs the secret key shown in Settings, compared in constant time,
 *    and after [AutomationApps.MAX_KEY_FAILURES] wrong keys in a minute
 *    every keyed request is refused until the minute is up;
 *  - it does only what the app's buttons do (the checks are
 *    [AutomationApps]): a device action that device lists (or a preset,
 *    before it has said), text, or an automation you made. Never this
 *    phone's own controls, except through an automation you set up;
 *  - at most [AutomationApps.RUNS_PER_MINUTE] things a minute, each with a
 *    notification saying which app asked for what, and every request in
 *    the history in Settings. The asking app is named only when Android
 *    says who sent it (Android 14+, if the app shares it; or the only app
 *    holding the permission); a name the app gives itself is never used.
 *
 * Requests (explicit broadcasts to `com.mydomain.android`, class
 * `com.mydomain.android.AutomationAppsKeyReceiver` or
 * `com.mydomain.android.AutomationAppsReceiver`):
 *   action RUN             extras device, action, args? (JSON object)
 *   action SEND_TEXT       extras device, text, as? (clipboard | notice)
 *   action RUN_AUTOMATION  extra automation (id or label)
 *   action LIST_DEVICES
 * plus `key` (or data `key:<key>`) for the key receiver. The answer is the
 * ordered broadcast's result (code RESULT_OK or 1, data JSON), and, when
 * the request has `reply_package`, a broadcast
 * [AutomationApps.ACTION_RESULT] to that package with extras ok, result
 * (JSON), error and request_id (echoed).
 */
open class AutomationAppsReceiver : BroadcastReceiver() {
    /** Must the request carry the key? */
    protected open val needsKey: Boolean = false

    override fun onReceive(context: Context, intent: Intent) {
        val app = context.applicationContext
        Core.init(app)
        AutomationAppsGate.load(app)
        val caller = sender(app)
        val reply = Reply(intent, isOrderedBroadcast)
        val sync = { ok: Boolean, json: String ->
            if (reply.ordered) setResult(if (ok) Activity.RESULT_OK else Activity.RESULT_FIRST_USER, json, null)
            reply.send(app, ok, json)
        }
        val work = AutomationAppsGate.take(app, intent, needsKey, caller, sync) ?: return
        val pending = goAsync()
        AutomationAppsGate.run(app, caller, work) { ok, json ->
            if (reply.ordered) pending.setResult(if (ok) Activity.RESULT_OK else Activity.RESULT_FIRST_USER, json, null)
            reply.send(app, ok, json)
            pending.finish()
        }
    }

    /** The package that sent this broadcast, as Android tells it ("" when
     *  it doesn't): Android 14+ says, if the sender chose to share it;
     *  else, here, the one app holding the permission is the only one
     *  that could have. */
    private fun sender(ctx: Context): String {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.UPSIDE_DOWN_CAKE) {
            runCatching { sentFromPackage }.getOrNull()?.takeIf { it.isNotEmpty() }?.let { return it }
        }
        if (!needsKey) AutomationAppsGate.allowedApps(ctx).singleOrNull()?.let { return it }
        return ""
    }

    /** Where the answer goes besides the ordered result. */
    private class Reply(intent: Intent, val ordered: Boolean) {
        private val pkg = intent.getStringExtra(AutomationApps.EXTRA_REPLY_PACKAGE)?.trim()
            ?.takeIf { it.length <= 255 && PACKAGE.matches(it) }
        private val requestId = intent.getStringExtra(AutomationApps.EXTRA_REQUEST_ID)?.take(200)
        private val request = intent.action

        fun send(ctx: Context, ok: Boolean, json: String) {
            val to = pkg ?: return
            val error = if (ok) null else runCatching { JSONObject(json).optString("error") }.getOrNull()
            val i = Intent(AutomationApps.ACTION_RESULT)
                .setPackage(to)
                .putExtra("ok", ok)
                .putExtra("result", json)
                .putExtra("request", request)
            error?.let { i.putExtra("error", it) }
            requestId?.let { i.putExtra(AutomationApps.EXTRA_REQUEST_ID, it) }
            runCatching { ctx.sendBroadcast(i) }
        }
    }

    private companion object {
        val PACKAGE = Regex("^[A-Za-z][A-Za-z0-9_]*(\\.[A-Za-z][A-Za-z0-9_]*)+$")
    }
}

/** The same requests, from any app that has the key (Tasker and the like). */
class AutomationAppsKeyReceiver : AutomationAppsReceiver() {
    override val needsKey: Boolean = true
}

/**
 * The switch, the key, the limits, the history and the running, for the
 * receivers and the Settings tile. The receivers call in on the main
 * thread; the device actions run off it.
 */
object AutomationAppsGate {
    private const val TAG = "mydomain_apps"
    const val PREF_ON = "automation_apps"
    private const val PREF_HISTORY = "automation_apps_history"
    private const val SECRET_KEY = "automation_apps_key"
    /** How long to wait for an answer before saying "sent". */
    private const val ANSWER_MS = 9_000L

    /** The history, newest first (main thread). */
    val history = mutableStateListOf<AppRun>()

    private var loaded = false
    @Volatile private var cachedKey: String? = null
    private val runs = RateWindow(AutomationApps.RUNS_PER_MINUTE, AutomationApps.WINDOW_MS)
    private val lists = RateWindow(AutomationApps.LISTS_PER_MINUTE, AutomationApps.WINDOW_MS)
    private val keyFailures = RateWindow(AutomationApps.MAX_KEY_FAILURES, AutomationApps.WINDOW_MS)
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
    private val noteId = AtomicInteger(9300)

    private fun prefs(ctx: Context) = ctx.getSharedPreferences("md", Context.MODE_PRIVATE)

    /** Read the history, once. Main thread. */
    fun load(ctx: Context) {
        if (loaded) return
        loaded = true
        history.clear()
        history.addAll(AutomationApps.decodeHistory(prefs(ctx).getString(PREF_HISTORY, null)))
    }

    fun enabled(ctx: Context): Boolean = prefs(ctx).getBoolean(PREF_ON, false)

    /** Switch it on or off; on for the first time makes the key. */
    fun setEnabled(ctx: Context, on: Boolean) {
        prefs(ctx).edit().putBoolean(PREF_ON, on).apply()
        if (on && key(ctx) == null) newKey(ctx)
    }

    /** The key for [AutomationAppsKeyReceiver] (null: none made yet). */
    fun key(ctx: Context): String? {
        cachedKey?.let { return it }
        return KeystoreSecrets(ctx.applicationContext).get(SECRET_KEY)?.takeIf { it.isNotEmpty() }?.also { cachedKey = it }
    }

    /** A new key (the old one stops working): 24 random bytes, URL-safe
     *  base64, kept sealed by the Android Keystore. */
    fun newKey(ctx: Context): String {
        val bytes = ByteArray(24).also { SecureRandom().nextBytes(it) }
        val k = Base64.encodeToString(bytes, Base64.URL_SAFE or Base64.NO_WRAP or Base64.NO_PADDING)
        KeystoreSecrets(ctx.applicationContext).set(SECRET_KEY, k)
        cachedKey = k
        return k
    }

    /** The apps Android has granted [AutomationApps.PERMISSION] (package names). */
    fun allowedApps(ctx: Context): List<String> = runCatching {
        ctx.packageManager.getPackagesHoldingPermissions(arrayOf(AutomationApps.PERMISSION), 0)
            .map { it.packageName }
            .filter { it != ctx.packageName }
    }.getOrDefault(emptyList())

    /** An app's name for people ("" or unknown: "An app on this phone"). */
    fun appName(ctx: Context, pkg: String): String {
        if (pkg.isEmpty()) return ctx.getString(R.string.automation_apps_unknown_app)
        return runCatching {
            val pm = ctx.packageManager
            pm.getApplicationLabel(pm.getApplicationInfo(pkg, 0)).toString()
        }.getOrNull()?.takeIf { it.isNotBlank() } ?: pkg
    }

    fun clearHistory(ctx: Context) {
        history.clear()
        save(ctx)
    }

    private fun record(ctx: Context, run: AppRun) {
        val list = AutomationApps.record(history.toList(), run)
        history.clear()
        history.addAll(list)
        save(ctx)
    }

    private fun save(ctx: Context) {
        prefs(ctx).edit().putString(PREF_HISTORY, AutomationApps.encodeHistory(history.toList())).apply()
    }

    /** Your devices, as requests name them (not this phone). */
    private fun peers(): List<AppPeer> {
        val me = Core.me?.nodeId
        return Core.peers.filter { it.nodeId != me }.map { p ->
            val nick = DeviceLabels.nickname(p.nodeId)
            AppPeer(p.nodeId, Core.peerName(p.nodeId), listOfNotNull(nick, p.name, niceName(p.name)).distinct())
        }
    }

    /** Something checked and ready to run off the main thread: [what] for
     *  people, [act] the doing (the transport used, "queued" or "here"). */
    class Work(val what: String, val sentence: String, val act: suspend () -> String)

    /**
     * Check a request (main thread). Answers it through [answer] and
     * returns null when it's refused or done here (a list); else the
     * [Work] to [run].
     */
    fun take(ctx: Context, intent: Intent, needsKey: Boolean, caller: String, answer: (Boolean, String) -> Unit): Work? {
        val now = System.currentTimeMillis()
        val refuse = { why: String -> answer(false, AutomationApps.resultJson(Result.failure(AppRefusal(why)))) }
        if (!enabled(ctx)) {
            refuse(ctx.getString(R.string.automation_apps_off))
            return null
        }
        if (needsKey) {
            if (keyFailures.full(now)) {
                refuse(ctx.getString(R.string.automation_apps_locked_out))
                return null
            }
            val given = AutomationApps.keyOf(intent.getStringExtra(AutomationApps.EXTRA_KEY), intent.dataString)
            if (!AutomationApps.keyMatches(key(ctx), given)) {
                keyFailures.hit(now)
                val why = ctx.getString(R.string.automation_apps_wrong_key)
                record(ctx, AppRun(now, caller, why, false, why))
                refuse(why)
                return null
            }
        }
        val request = try {
            AutomationApps.parse(intent.action) { k -> runCatching { intent.getStringExtra(k) }.getOrNull() }
        } catch (e: AppRefusal) {
            record(ctx, AppRun(now, caller, ctx.getString(R.string.automation_apps_bad_request), false, e.message.orEmpty()))
            refuse(e.message.orEmpty())
            return null
        }
        if (request == AppRequest.ListDevices) {
            if (!lists.hit(now)) {
                refuse(ctx.getString(R.string.automation_apps_too_many))
                return null
            }
            val list = peers()
            val json = AutomationApps.devicesJson(list, Core.links.keys.toSet()) { Core.deviceStatus[it]?.actions }
            record(ctx, AppRun(now, caller, ctx.getString(R.string.automation_apps_what_list), true, ""))
            answer(true, json)
            return null
        }
        val who = appName(ctx, caller)
        val work = try {
            if (runs.full(now)) throw AppRefusal(ctx.getString(R.string.automation_apps_too_many))
            prepare(ctx, request, who)
        } catch (e: AppRefusal) {
            record(ctx, AppRun(now, caller, describe(ctx, intent), false, e.message.orEmpty()))
            refuse(e.message.orEmpty())
            return null
        }
        runs.hit(now)
        return work
    }

    /** A short "what" for a request that couldn't be checked. */
    private fun describe(ctx: Context, intent: Intent): String =
        intent.action?.substringAfterLast('.')?.takeIf { it.isNotEmpty() }
            ?: ctx.getString(R.string.automation_apps_bad_request)

    /** [request] resolved and checked; throws [AppRefusal]. */
    private fun prepare(ctx: Context, request: AppRequest, who: String): Work {
        val running = Core.engine() != null
        return when (request) {
            is AppRequest.Run -> {
                if (!running) throw AppRefusal(ctx.getString(R.string.automation_apps_not_running))
                val p = AutomationApps.findDevice(peers(), request.device)
                AutomationApps.checkAction(request.action, Core.deviceStatus[p.node]?.actions, p.display)
                if (request.action == "ring") {
                    Work(
                        ctx.getString(R.string.automation_apps_what_ring, p.display),
                        ctx.getString(R.string.automation_apps_asked_ring, who, p.display),
                    ) { Core.ring(p.node) }
                } else {
                    val args = JSONObject(request.args)
                    Work(
                        ctx.getString(R.string.automation_apps_what_run, request.action, p.display),
                        ctx.getString(R.string.automation_apps_asked_run, who, request.action, p.display),
                    ) { Core.runAction(p.node, request.action, args) }
                }
            }
            is AppRequest.SendText -> {
                if (!running) throw AppRefusal(ctx.getString(R.string.automation_apps_not_running))
                val p = AutomationApps.findDevice(peers(), request.device)
                val clip = request.mode == "clipboard"
                Work(
                    ctx.getString(if (clip) R.string.automation_apps_what_clipboard else R.string.automation_apps_what_notice, p.display),
                    ctx.getString(if (clip) R.string.automation_apps_asked_clipboard else R.string.automation_apps_asked_notice, who, p.display),
                ) { Core.sendText(p.node, request.text, request.mode) }
            }
            is AppRequest.RunAutomation -> {
                val a = AutomationApps.findAutomation(PhoneAutomations.load(ctx).list, request.automation)
                if (!running && a.node.isNotEmpty() && a.action != AutomationBook.NOTIFY) {
                    throw AppRefusal(ctx.getString(R.string.automation_apps_not_running))
                }
                val name = a.label.ifBlank { PhoneAutomations.thenText(ctx, a) }
                Work(
                    ctx.getString(R.string.automation_apps_what_automation, name),
                    ctx.getString(R.string.automation_apps_asked_automation, who, name),
                ) { PhoneAutomations.act(ctx, a) }
            }
            AppRequest.ListDevices -> throw AppRefusal(ctx.getString(R.string.automation_apps_bad_request))
        }
    }

    /** Do [work] off the main thread, at most [ANSWER_MS] before answering
     *  (a slow device: "pending", and it goes on); then the history, a
     *  notification and [done] (any thread). */
    fun run(ctx: Context, caller: String, work: Work, done: (Boolean, String) -> Unit) {
        val now = System.currentTimeMillis()
        scope.launch {
            val job = scope.async { work.act() }
            val result = withTimeoutOrNull(ANSWER_MS) { runCatching { job.await() } } ?: Result.success("pending")
            val shown = result.fold({ Result.success(it) }, { Result.failure<String>(AppRefusal(friendlyError(it))) })
            withContext(Dispatchers.Main) {
                record(ctx, AppRun(now, caller, work.what, shown.isSuccess, shown.fold({ it }, { it.message.orEmpty() })))
                tell(ctx, work.sentence, shown)
            }
            done(shown.isSuccess, AutomationApps.resultJson(shown))
        }
    }

    /** "Tasker asked to ring Work Mac": done, waiting, or why not. */
    private fun tell(ctx: Context, sentence: String, result: Result<String>) {
        val body = statusText(ctx, result.isSuccess, result.fold({ it }, { it.message.orEmpty() }))
        PhoneAutomations.createChannel(ctx)
        val open = Intent(ctx, MainActivity::class.java)
            .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_SINGLE_TOP)
            .putExtra(PhoneLocation.EXTRA_OPEN_SETTINGS, true)
        val id = noteId.incrementAndGet()
        val pi = PendingIntent.getActivity(ctx, id, open, PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
        val n = NotificationCompat.Builder(ctx, PhoneAutomations.CHANNEL)
            .setSmallIcon(android.R.drawable.ic_dialog_info)
            .setContentTitle(sentence)
            .setContentText(body)
            .setStyle(NotificationCompat.BigTextStyle().bigText(body))
            .setPriority(NotificationCompat.PRIORITY_DEFAULT)
            .setAutoCancel(true)
            .setContentIntent(pi)
            .build()
        runCatching { ctx.getSystemService(NotificationManager::class.java)?.notify(id, n) }
            .onFailure { Log.w(TAG, "can't post the notification", it) }
    }

    /** How a run went, for people: done, waiting to reach the device, sent
     *  with no answer yet, or why it couldn't. */
    fun statusText(ctx: Context, ok: Boolean, detail: String): String = when {
        !ok -> ctx.getString(R.string.automation_apps_failed, detail)
        detail == "queued" -> ctx.getString(R.string.automation_apps_queued)
        detail == "pending" -> ctx.getString(R.string.automation_apps_pending)
        else -> ctx.getString(R.string.automation_apps_done)
    }
}
