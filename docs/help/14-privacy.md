---
title: Privacy and encryption
keywords: privacy, password, password manager, concealed, backup, back up, restore, settings backup, new computer, reinstall, private, encryption, encrypted, end-to-end, server, security, safe, consent, permission, key, device key, data, what is sent
platforms: desktop, android
---
## End-to-end encrypted

Everything your devices send each other (messages, clipboard, files, notifications, screen pictures, audio, locations) is encrypted with your **device key** before it leaves the device. Only your own devices, signed in with that key, can read it.

## What the server sees

The my-domain server helps your devices find each other and, when they can't connect directly, passes their traffic along. It knows:

- your username and your list of devices, with their names and network addresses;
- when devices are online, and when one wakes another.

It can't read what you send: it never has your device key. Messages are not stored on the server.

## Consent: the device being reached decides

- **my-domain never grants permissions itself.** Each one is turned on by you, in Android's or your computer's own settings.
- Sensitive things are off until you turn them on, on the device they concern: sharing a location, letting your devices see a screen, calls on the computer, webhooks.
- Screen sharing and phone audio are asked for every time, by Android itself.
- A notification tells you when a device's location is shared, when its screen is being watched, and when an automation or webhook runs.

## Copied passwords

A password copied from a password manager is never synced to your other devices or kept in a clipboard history, and if you send it on purpose the other device keeps it out of its history and notifications too. On the computer you can also exclude apps whose copies should never be synced. See [Messages, clipboard and clipboard history](help:messages-clipboard).

## What stays on each device

Clipboard history, notification history, transfer history and battery history are kept on the device that shows them, not on the server. You can clear or turn each off in Settings.

## Backing up your settings

**Settings → Back up settings** saves this app's settings to a file you choose, and **Restore settings…** brings them back after a reinstall or on a new computer or phone. Before anything changes you see what each part would change, and pick which parts to restore. Parts that name devices that aren't linked on this device yet start unticked.

What the backup holds: your preferences and switches, theme, automations, scheduled actions, commands, keyboard shortcuts, webhook on/off and port, device names and icons, page layouts, sharing switches, battery alerts and which folders are synced with which device (not the files in them).

What it never holds: your password, device key or any encryption key, your sign-in, the webhook token or the automation-apps key, push-notification credentials, or the content of any message, notification, clipboard item or note. The file isn't encrypted, so keep it somewhere you trust all the same: it lists your devices' names and your folder paths.

A backup from a computer restores on a computer, one from the phone on the phone. Most restored settings take effect when the app restarts; it offers to restart.
