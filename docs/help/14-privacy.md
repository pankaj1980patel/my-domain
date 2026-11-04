---
title: Privacy and encryption
keywords: privacy, private, encryption, encrypted, end-to-end, server, security, safe, consent, permission, key, device key, data, what is sent
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

## What stays on each device

Clipboard history, notification history, transfer history and battery history are kept on the device that shows them, not on the server. You can clear or turn each off in Settings.
