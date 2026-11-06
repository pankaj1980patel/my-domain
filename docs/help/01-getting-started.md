---
title: Getting started and pairing
keywords: sign in, account, create account, pair, add device, add a device, join, join with a code, qr, qr code, scan, device key, password, username, first time, setup, connect, icon, device icon, change icon, model, laptop, desktop, tablet, foldable, first run, setup, set up, name this device, rename, choose features, health check, health
platforms: desktop, android
---
my-domain links your own phone and computers so they can send things to each other and do small jobs for each other: share the clipboard, send files, ring a lost phone, show notifications on the computer, and more.

## Sign in on every device

1. Install my-domain on your phone and on each computer.
2. On the first one, choose **Create account** and pick a username and password.
3. Fill in a **device key**, or tap **Generate** for one. Write it down somewhere safe.
4. On every other device, choose **Sign in** with the same username, password **and the same device key**.

There is no separate pairing step: devices signed in to the same account find each other on their own and show up under **Devices**.

## First-run setup

Right after you sign in (or join with a code), a short setup opens. Five steps, each one skippable:

1. **Name this device.** It starts with the device's name or model; your other devices list it by what you type.
2. **Add your other devices.** The same QR code and 6-digit code as **Settings → Add a device**. Already added? Go on.
3. **Choose what to use:** Clipboard, Notifications, Files, Audio and screen, Find and ring, each with a line on what it does. Clipboard sync follows your pick; leaving a group out turns its sharing off. Nothing that shares more is turned on behind your back.
4. **Allow what that needs.** Only the permissions the groups you picked need. Each button opens the system's own prompt or page: my-domain never turns anything on by itself.
5. **Test the connection.** Live: how this device reaches each of your others (local network, directly over the internet, or only through the relay) and how long the last reconnect took.

**Later** closes it; Home shows **Finish setting up** and carries on at the same step. **Skip setup** ends it. Each device remembers its own setup, and **Settings → Health check → Run setup again** starts it over.

## Health check

**Settings → Health check** lists everything my-domain needs, each with a coloured dot and one button that fixes it: green is fine, grey is just so you know, amber is worth a look, red stops something from working. It checks permissions for the features you use, battery and autostart on a phone, Local Network, Accessibility and Screen Recording on a Mac, the server, push messages, how each device is linked (and why only through the relay), security keys that changed, space for received files, and whether a device runs an older app that lacks features. When something is red, Home and the sidebar (Settings on the phone) say so, and so does that device's page. See [Troubleshooting](help:troubleshooting).

## Add a device without typing

On a device that's already signed in, open **Settings → Add a device**. It shows a QR code and a 6-digit code under it.

- **A phone:** point its camera (or Google Lens) at the QR code and open the link. my-domain asks whose account it joins; tap **Join**.
- **A computer, or a phone that can't scan:** on its sign-in screen choose **Join with a code** and type the 6 digits.

The new device is signed in to the same account with the same device key, without your password, and the first device says it joined. The code works **once**, for **5 minutes**; closing it cancels it. See [Privacy](help:privacy) for what the code contains.

If it says to update the server, the server is older than this feature: sign in with your username, password and device key instead.

## The device key

The device key is what encrypts everything your devices send each other. The server never sees it.

- Every device must use the same key. A device with a different key can't read the others.
- If you change it in Settings, change it on every device too.
- If you lose it, sign in again everywhere with a new one.

## Device icons

Each device gets an icon for what it is, worked out on its own: a Mac laptop, an iMac or Mac desktop, a Windows or Linux laptop or desktop, an Android phone, tablet or foldable, or a TV. Its page also says the model, like “Apple MacBook Pro” or “Samsung Galaxy S23”. A device with an older app shows a plain icon until it's updated.

To pick another icon, open the device's page, choose **Edit** and pick one under **Icon** (**Auto** goes back to the automatic one). While **Share names and icons with my other devices** is on in Settings, your other devices show it too.

## Connected or not

Each device shows whether it is connected right now: directly (same network, fastest), through the internet, or not connected. Things you send to a device that isn't connected wait and go when it comes back, where that makes sense (a ring or a message, not a live screen).

## Next steps

- Open **Settings → Health check** to see what works and fix what doesn't.
- On the phone, open **Settings → Set up this phone** to see every optional permission and what each is for. See [Set up this phone](help:setup-phone).
- If two devices don't see each other, see [Troubleshooting](help:troubleshooting).
