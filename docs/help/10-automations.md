---
title: Automations, schedules, webhooks and shortcuts
keywords: automation, automations, schedule, schedules, timer, trigger, when, webhook, webhooks, home assistant, token, api, keyboard, shortcut, shortcuts, hotkey, command, run command, script
platforms: desktop, android
---
## Automations

"When this happens, do that." For example: when my phone disconnects, lock this computer; when the laptop's battery drops below 20%, tell me.

- **On the computer** (Settings → Automations): when a device connects, disconnects (for a minute), its battery drops below a level, it starts or stops charging, or this computer wakes from sleep.
- **On the phone** (Settings → Automations): when a device connects or disconnects, the phone's battery drops or charging changes, or it joins or leaves Wi-Fi.
- The result can be an action on any of your devices, or just a notification.

Each one runs at most once a minute and tells you when it runs. Automations only work while my-domain is running.

## Schedules (computer)

Run an action at a set time, once or on chosen weekdays, from a device's page or for this computer in Settings. A time missed while the computer was asleep or off is skipped, not run later.

## Commands (computer)

In the computer's Settings you can write named commands that your other devices may run as buttons. Only the name you chose is sent; the command itself stays on the computer. Use it with care.

## Webhooks (computer)

For tools on the same computer, such as Home Assistant or a script. Off by default. When on:

- It only listens on this computer (`127.0.0.1`), never on the network.
- Every request needs the secret token shown in Settings. **Make a new token** if it ever leaks.
- Too many wrong tokens locks it out for a while.
- You get a notification each time it is used.

## Keyboard shortcuts (computer)

In Settings → Keyboard shortcuts, bind your own keys to: send the clipboard, ring, lock, locate, send a file, mute, play/pause, open the command palette (Cmd+K or Ctrl+K), show or hide my-domain, start or stop a phone's audio, or run an automation. my-domain warns you if a combination is reserved or already used.
