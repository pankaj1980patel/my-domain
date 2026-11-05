---
title: Troubleshooting
keywords: problem, help, not working, can't connect, offline, not connected, firewall, relay, internet, slow, check connection, speed test, ping, round trip, unreachable, battery optimisation, background, wi-fi, network, router, vpn, company network, sign in, key
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

**Test speed** sends a few megabytes each way over the current connection and shows how fast it went, both ways, in Mbit/s, the same way files travel. It only works while connected, and you can cancel it at any time. Measuring needs my-domain to be up to date on both devices; if the other one is older, the check says so.

## The phone is slow to answer, or misses things when the screen is off

Android puts apps to sleep to save battery. In Set up this phone, turn on **Unrestricted battery use** and choose "Don't optimise" for my-domain. Some phone makers have an extra battery manager of their own; allow my-domain to run in the background there too.

## A button is greyed out

Hover over it (or read the line under it): it says why, for example a permission that is off on the other device, or a feature that device doesn't have.

## A feature needs a permission

On the phone, see [Set up this phone](help:setup-phone). On a Mac, check System Settings → Privacy & Security (Accessibility, Screen Recording, Location) for my-domain.

## Sign-in problems

Check the username and password. On a company network that inspects secure connections, my-domain uses your computer's trusted certificates, so it should work if your browser does.
