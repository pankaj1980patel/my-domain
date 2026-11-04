---
title: Troubleshooting
keywords: problem, help, not working, can't connect, offline, not connected, firewall, relay, internet, slow, battery optimisation, background, wi-fi, network, router, vpn, company network, sign in, key
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

## The phone is slow to answer, or misses things when the screen is off

Android puts apps to sleep to save battery. In Set up this phone, turn on **Unrestricted battery use** and choose "Don't optimise" for my-domain. Some phone makers have an extra battery manager of their own; allow my-domain to run in the background there too.

## A button is greyed out

Hover over it (or read the line under it): it says why, for example a permission that is off on the other device, or a feature that device doesn't have.

## A feature needs a permission

On the phone, see [Set up this phone](help:setup-phone). On a Mac, check System Settings → Privacy & Security (Accessibility, Screen Recording, Location) for my-domain.

## Sign-in problems

Check the username and password. On a company network that inspects secure connections, my-domain uses your computer's trusted certificates, so it should work if your browser does.
