---
title: Troubleshooting
keywords: problem, help, reconnect, reconnect time, asleep, waking, push, autostart, doze, drops, dropped, how connected, connections map, map, links, why not connected, upnp, bluetooth, not working, can't connect, offline, not connected, firewall, relay, internet, slow, check connection, speed test, ping, round trip, unreachable, battery optimisation, background, wi-fi, network, router, vpn, company network, sign in, key, security key, key changed, reinstall, reinstalled, cleared data, trust new key
platforms: desktop, android
---
## A device doesn't appear

- Check that both are signed in to the **same account** with the **same device key**. A device with a different key can't read the others.
- Pull down to refresh the list on the phone, or wait a few seconds.

## It says "not connected"

Press **Connect** on the device. my-domain tries, in order: a direct link on the same network, a direct link across the internet, and finally passing the traffic through its server (**Connected through the internet**). Through the server it still works and stays encrypted, only more slowly.

If two devices on the same Wi-Fi only connect through the internet:

- **Computer firewall:** allow my-domain to accept incoming connections. On a Mac: System Settings → Network → Firewall, and Privacy & Security → Local Network. On Windows, allow it when Windows asks, on private networks.
- **Router:** some routers (often guest networks) stop devices on the same Wi-Fi from talking to each other ("client isolation"). Try the main network.
- **VPN or company network:** these can block direct links. The server route usually still works.

## Check why a device is slow or unreachable

Open the device and find the **Connection** tile (on the phone, on a computer's screen). Press **Check connection** and my-domain tells you, in plain words, how it reaches that device:

- **Connected directly on your home network**: the best case. Files and screen sharing go as fast as your Wi-Fi allows.
- **Connected directly over the internet**: also good; speed depends on both internet connections.
- **Going through the relay server**: it works and stays encrypted, but every byte makes a detour through the server, so it's slower. Putting both devices on the same Wi-Fi usually fixes it; so does allowing my-domain through the computer's firewall (see above).
- **Not reachable**, with when it last tried and why: for example, the phone didn't answer the push (it may be switched off, offline, or have my-domain put to sleep by battery saving), or the relay server couldn't be reached (check your internet connection).

Below that are the details: the round trip in milliseconds (measured just now, while the devices are connected), the addresses each device has, whether each network is "open" or "strict" (two strict networks can only meet through the relay server), whether the relay server answers, and the last few connection attempts.

The details also say how the connection holds up over time:

- **Reconnect time**: how long the last reconnect took, and the average of the recent ones. It's timed from what started it: this device's network changing (say, Wi-Fi to mobile data), a push waking this device, or this device pushing the other one awake. A few seconds is normal; much longer usually means the phone's battery saving held it back (see below).
- **Up for** / **Links last**: how long the current connection has been up, and how long the recent ones lasted.
- **Last dropped**, and why: closed here or there because it wasn't used (a phone in the background does that to save battery, and is woken by a push when needed), the network changed, it went silent, or it was closed by hand.

**Test speed** sends a few megabytes each way over the current connection and shows how fast it went, both ways, in Mbit/s, the same way files travel. It only works while connected, and you can cancel it at any time. Measuring needs my-domain to be up to date on both devices; if the other one is older, the check says so.

## See how all your devices are connected

On the Devices page, choose **How they're connected** (on a computer it's also in the command palette). This device sits in the middle with your other devices around it, and each line says how they're linked:

- **Local network** (green): directly on your Wi‑Fi or home network, the fastest.
- **Direct over the internet** (blue): straight across the internet, through both routers if needed.
- **Relay server** (amber): through my-domain's server. It works and stays end-to-end encrypted (the server can't read it), just slower.
- **Bluetooth** (purple, dotted): close by with no shared network; fine for messages, slow for files.
- **Not connected** (grey, dashed), with when the device was last seen.

Where it's known, the round trip shows next to the line. Lines between your *other* devices (say, your phone and your laptop) appear too, once those devices are connected here and up to date: each one tells the others how it is linked. Tap a device or a line for the details (the path, the address and what kind it is, since when, the encryption) and **Connection details** to open that device's Connection tile. The same is listed as text under the picture, for screen readers.

For a device that isn't connected, or only through the relay or Bluetooth, the details say **why**, per way it was tried, with what you can do:

- **Local network**: it's on a different network, nothing here saw it, or it's here but didn't answer (a firewall or the router's client isolation; see above).
- **Internet, directly**: the device didn't answer (asleep, off or offline), both networks are strict (only the relay can join them), getting through the routers timed out, or neither router opens a port (turn on UPnP or NAT-PMP).
- **Relay server**: not signed in, the server can't be reached, it refused the sign-in (sign out and in again), it's full, or the device didn't answer through it.
- **Bluetooth**: it's off, not paired, or out of range.
- **The device**: what it sends can't be read, because it uses a different device key; or its **security key changed** (see below).

## "Its security key changed"

Each install of my-domain has a security key of its own, and your devices remember the one each device used the first time. When a device comes back with another key, they refuse to connect to it directly, because it could be someone else using its name. This happens when the app is **reinstalled** or its **data is cleared**.

You'll get a notification once, and the device's page (and its details on the connections map) says that its **security key changed**:

- If you reinstalled the app on it or cleared its data, choose **Trust the new key**. You're shown the old and new keys' short fingerprints first; only that new key is trusted, and it connects directly again.
- If you didn't, **don't trust it**: choose **Not now**. It stays refused (it may still reach you through the relay server, end-to-end encrypted with your device key) and you won't be asked again, unless yet another key turns up.

Trusting is never automatic, and a key that changes again later is refused again.

## "Asleep" or "Waking it…"

A phone in the background closes connections it isn't using, so it doesn't wake its radio all day. It's still reachable: when a computer needs it, the computer sends it a push, which wakes it, and it connects back within a few seconds. While that happens the computer shows **Waking it…**.

**Asleep** means the phone didn't answer its last push, or closed its unused connection itself. It isn't an error: the computer doesn't keep pushing a sleeping phone by itself (it waits longer each time it gets no answer), but anything you do with the phone (sending a file, a message, pressing **Connect**) wakes it right away. If a phone is asleep every time, check its battery settings below.

When the phone's network changes (a new Wi-Fi network, Wi-Fi to mobile data and back), it reconnects to your computers at once, and keeps trying for a couple of minutes if the network isn't ready yet. When it comes home to the same Wi-Fi as a computer, a connection that went through the internet moves to the home network by itself.

## The phone is slow to answer, or misses things when the screen is off

Android puts apps to sleep to save battery. In Set up this phone:

1. Turn on **Unrestricted battery use** and choose "Don't optimise" (or **Unrestricted**) for my-domain.
2. On vivo, iQOO, Xiaomi, OPPO, realme, OnePlus, Samsung and some other phones, also open **Start by itself (phone maker's setting)**: allow my-domain to start by itself and to run in the background there. These phones have a battery manager of their own that otherwise stops my-domain after a while, whatever Android allows.

my-domain starts again by itself after the phone restarts or the app is updated, and checks every quarter of an hour that it's still running. If the phone stopped it anyway, it starts it again.

## A button is greyed out

Hover over it (or read the line under it): it says why, for example a permission that is off on the other device, or a feature that device doesn't have.

## A feature needs a permission

On the phone, see [Set up this phone](help:setup-phone). On a Mac, check System Settings → Privacy & Security (Accessibility, Screen Recording, Location) for my-domain.

## Sign-in problems

Check the username and password. On a company network that inspects secure connections, my-domain uses your computer's trusted certificates, so it should work if your browser does.

## Updates

On a computer, my-domain looks for a new version 30 seconds after it starts and then every 6 hours. It never installs one without asking unless you tell it to.

- When there's a new version, a banner says **Version X is available**. **What's new** shows the release notes, **Install and restart** downloads it (you see the progress), installs it and starts the new version, and **Later** hides the banner until my-domain starts again.
- **Settings → Updates** shows the version you have, **Check now**, and **Install updates automatically** (off at first). With it on, a new version downloads in the background and installs the next time you quit my-domain (or press **Restart now**).
- Only stable releases are offered. Every download is checked against my-domain's signing key before it's installed; one that doesn't match is refused.
- No internet, or the update server not answering: nothing pops up, Settings → Updates just says it couldn't check, and it tries again later.
- "This build can't update itself": it was built without the update key (or it's a development build). Download the new version from the releases page instead.
- On Linux, the **AppImage** replaces itself; a **.deb** install asks for your password to install the new version.
- On a Mac the app may ask for your password to replace itself if it's in a folder you can't write to. Unsigned builds may ask for permissions (Bluetooth, microphone, location) again after an update.

Phones update through their app store or by installing the new APK.
