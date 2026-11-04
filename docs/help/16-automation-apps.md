---
title: Automation apps (Tasker and others)
keywords: tasker, macrodroid, automate, automation app, intent, broadcast, send intent, key, permission, routine, llama, ifttt
platforms: android
---
Apps like Tasker, MacroDroid or Automate can ask my-domain to ring or lock your computer, send it text, or run one of your phone automations. It is the phone's version of the computer's webhooks.

## Turn it on

In **Settings → Automation apps**, switch on **Let automation apps on this phone use my-domain**. It is off until you do, and while it is off every request is refused.

## What keeps it safe

- An app needs either the **key** shown in Settings, or my-domain's **automation permission**, which Android asks you about for each app that wants it. Keep the key private, and tap **New key** if it ever leaks.
- Too many wrong keys and my-domain stops listening for a minute.
- It can only do what my-domain's buttons do: an action the device offers, text, or an automation you made. It never changes this phone's own settings, except through an automation you set up yourself.
- At most 6 requests a minute, each with a notification saying which app asked for what.
- **History** in Settings lists the last 50 requests. Android names the app only in some cases (Android 14 and the app shares its name, or the only app with the permission); otherwise it says "An app on this phone".

## Use it with Tasker

Add the action **System → Send Intent** and fill it in (Settings has examples ready to copy, with your key):

- **Action**: `com.mydomain.android.action.RUN`
- **Package**: `com.mydomain.android`
- **Class**: `com.mydomain.android.AutomationAppsKeyReceiver`
- **Target**: Broadcast Receiver
- **Data**: `key:` followed by your key
- **Extras**: `device:My computer` and `action:lock`

Package and Class must be filled in exactly: nothing else on the phone sees the request. MacroDroid and Automate have the same fields.

## The requests

- `com.mydomain.android.action.RUN`: extras `device`, `action` (like `ring`, `lock`, `mute`, `media`) and optionally `args`, a JSON object such as `{"on":true}`.
- `com.mydomain.android.action.SEND_TEXT`: extras `device`, `text`, and `as`: `clipboard` (the default) or `notice`.
- `com.mydomain.android.action.RUN_AUTOMATION`: extra `automation`, its name or id. It must be switched on.
- `com.mydomain.android.action.LIST_DEVICES`: your devices, with what each can do.

The device is its name as my-domain shows it, a nickname you gave it, or its node id. If two devices have the same name, use a nickname or the node id.

## Getting the answer

Add the extra `reply_package` with your automation app's package name, and listen for the intent `com.mydomain.android.event.RESULT` (in Tasker: the **Intent Received** event). It carries `ok`, `result` (JSON), `error` and your `request_id` if you sent one. Apps that send ordered broadcasts also get the answer as the broadcast's result.

## Limits

- my-domain must be running in the background. If Android stopped it, open it once.
- Requests for this phone itself aren't taken: use your automation app's own actions for that.
- A device that is asleep or offline gets the request once it is reachable.
