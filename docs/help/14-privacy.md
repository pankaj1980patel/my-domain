---
title: Privacy and encryption
keywords: privacy, permissions, per device, allow once, allow control, see this screen, remote control, verification code, otp, one-time code, password, password manager, concealed, backup, back up, restore, settings backup, new computer, reinstall, private, encryption, encrypted, end-to-end, server, security, safe, consent, permission, key, device key, data, what is sent, qr, qr code, join code, add a device, security key, key changed, fingerprint, trust, verify, verified, safety number, remove device, removed devices, revoke, lost device, stolen, allow again, signed, signature
platforms: desktop, android
---
## End-to-end encrypted

Everything your devices send each other (messages, clipboard, files, notifications, screen pictures, audio, locations) is encrypted with your **device key** before it leaves the device. Only your own devices, signed in with that key, can read it.

## What the server sees

The my-domain server helps your devices find each other and, when they can't connect directly, passes their traffic along. It knows:

- your username and your list of devices, with their names and network addresses;
- when devices are online, and when one wakes another.

It can't read what you send: it never has your device key. Messages are not stored on the server.

## Adding a device with a QR code

The QR code under **Settings → Add a device** is a one-time join link. It holds:

- the server's address and your username;
- a random one-time secret, which the server never sees (it is only told a one-way hash of it);
- your device key, locked with that secret.

It **never contains your password**. It signs in **one** device, only within **5 minutes**, and stops working once used or when you close it. Anyone who photographs it in those minutes could join your account, so show it only to your own devices, and close it once your device has joined.

The **6-digit code** is for a device that can't scan. So that it can bring the device key too, the server keeps a copy of the key locked with the code until it's used or expires (at most 5 minutes). Six digits are short, so scanning is the more private way: then the key never reaches the server at all. Wrong codes are limited, so they can't be guessed one after another.

## When a device's security key changes

Besides the device key you share, each install has a security key of its own that proves which device is on the other end of a direct connection. Your devices remember it the first time they meet. If a device turns up with another key (the app was reinstalled, its data cleared, or someone is pretending to be it), direct connections to it are refused until you choose **Trust the new key** on its page, after comparing the old and new fingerprints. Nothing is trusted by itself, and only the exact key you were shown is. Before you trust it, compare the new fingerprint with the one the other device shows under **Settings → This device → Your security key**: they must be the same. If the device was verified before, the warning says so: be extra careful. See [Troubleshooting](help:troubleshooting).

## Verifying a device

To be sure a device is really yours and nothing sits between them, verify it once:

1. Each device shows its own security key as a short fingerprint (four groups of four characters) in **Settings → This device → Your security key**, and under **Add a device**.
2. On a device's page, choose **Verify**. You see this device's fingerprint and the other one's side by side, and a **safety number** (12 groups of 5 digits) made from both keys. Open **Verify** for this device on the other one too: the safety number is the same on both when the keys are genuine.
3. If they match, choose **They match: mark as verified**. Or, on the phone, point the camera at the QR code the computer shows under **Verify** and open the link: the phone checks both keys itself and marks the computer verified. A code made for another device, from a device the phone doesn't know, or with keys that don't match is refused.

Verified devices show a **Verified** badge on their page and on the connection map. If a verified device's key ever changes, the badge goes away, and the key-changed warning says it was verified before. Verify it again after you've checked and trusted the new key.

## Messages are signed by the device that sent them

Your devices also sign every message with their own security key, so a message says which of your devices sent it, and that can't be faked: not by the server passing it along, not over Bluetooth, and not by another of your devices (one that was stolen, say). A message whose signature doesn't match the sender's key is refused, and so is an unsigned one from a device that signs. Older versions of the app don't sign; your devices still talk to them as before until they update. **Connection** on a device's page shows whether its messages are signed, and how many were refused.

## Removing a device

If you lose a device, sell it or stop trusting it, open its page and choose **Remove device**. It's removed from all your devices: they forget it, close any connection to it, and refuse it from then on, on your Wi-Fi, through the server and over Bluetooth, even though it still has your device key. This is **permanent for that install** of the app: the removal is signed by the device you made it on and kept on every device of yours, so it isn't found and added back.

- **Settings → Removed devices** lists them, with who removed each. **Allow again** takes a removal back on all your devices (only do this for a device you have back and trust).
- A removed device that still has your device key could join again as a **new** device (with a QR code or a code, or after a reinstall). It then shows up in your list as a new device, and you can remove that too. To keep it out for good, also choose **Change device key** in Settings on your other devices.

## Consent: the device being reached decides

- **my-domain never grants permissions itself.** Each one is turned on by you, in Android's or your computer's own settings.
- Sensitive things are off until you turn them on, on the device they concern: sharing a location, letting your devices see a screen, calls on the computer, webhooks.
- Screen sharing and phone audio are asked for every time, by Android itself. The camera is asked for every time too, on the phone, before anything is shown.
- A notification tells you when a device's location is shared, when its screen is being watched or its camera is on, and when an automation or webhook runs.

## Permissions: allowed device by device

Three things let another device act on a computer or see it. Each is **off** until you allow it, **for one device at a time**:

- **Allow <device> to control this computer**: its touchpad and keyboard move the pointer and type.
- **Allow <device> to see this screen**: it shows this screen.
- **Allow <device> to take over mouse & keyboard**: with Share mouse and keyboard, its pointer comes across onto this screen.

Find them on the device's page, under **Permissions**. Settings sums up who is allowed what, and the health check lists each device allowed anything (grey: nothing to fix, just worth knowing). They're kept in the settings backup.

- A device that isn't allowed is **refused**, and it says so: "Mac hasn't allowed control from this phone. Allow it on Mac."
- While my-domain runs on the computer, it asks you: "Pixel wants to control this computer": **Allow once** (until it stops, or 10 minutes unused), **Always**, or **Don't allow** (not asked again for a while). Nothing is ever allowed without your answer.
- Turning one off works at once: a device watching the screen stops seeing it, and a computer controlling this one's mouse and keyboard lets go.
- A removed device loses everything it was allowed; added again, it starts with nothing.
- On the phone, a computer may see the phone's screen when you share it from the computer's page (that counts as allowing it once), or always if you turn on **Allow <computer> to see this screen** there. Android still asks every time.

**After updating:** these used to be one switch each, on for all your devices. The update turns them off and shows, once, what was on, with an **Allow for <device>** button for each device. Allow only the ones you want.

## Copied passwords

A password copied from a password manager is never synced to your other devices, kept in a clipboard history or saved to [Read later](help:read-later), and if you send it on purpose the other device keeps it out of its history and notifications too. On the computer you can also exclude apps whose copies should never be synced. See [Messages, clipboard and clipboard history](help:messages-clipboard).

## Verification codes

**Send verification codes to my computers** is off until you turn it on on the phone. Then only four things go, end-to-end encrypted like every message: the code, the app it came from, who it's from (when the text says, like "Google") and when it expires. Never the notification's text. They go only to the computers you chose, only over a live link (a code waits at most 2 minutes for one), and the computer forgets the code after 5 minutes. Codes are never logged, never kept in Activity, a history or search, and a copied one is marked private and cleared from the clipboard after 90 seconds. The phone reads them from notifications only: never from your SMS. Private mode for the notification itself works as before. See [Phone notifications on the computer](help:notifications).

## What stays on each device

Clipboard history, notification history, transfer history and battery history are kept on the device that shows them, not on the server. You can clear or turn each off in Settings.

## Backing up your settings

**Settings → Back up settings** saves this app's settings to a file you choose, and **Restore settings…** brings them back after a reinstall or on a new computer or phone. Before anything changes you see what each part would change, and pick which parts to restore. Parts that name devices that aren't linked on this device yet start unticked.

What the backup holds: your preferences and switches, theme, automations, scheduled actions, commands, keyboard shortcuts, webhook on/off and port, device names and icons, page layouts, sharing switches, battery alerts and which folders are synced with which device (not the files in them).

What it never holds: your password, device key or any encryption key, your sign-in, the webhook token or the automation-apps key, push-notification credentials, or the content of any message, notification, clipboard item, note or saved link. The file isn't encrypted, so keep it somewhere you trust all the same: it lists your devices' names and your folder paths.

A backup from a computer restores on a computer, one from the phone on the phone. Most restored settings take effect when the app restarts; it offers to restart.
