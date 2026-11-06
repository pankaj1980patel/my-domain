---
title: Sending files and folders
keywords: file, files, print, printer, folder, send, transfer, transfers, resume, download, download on, link, url, web, share, share sheet, finder, explorer, file manager, drag, drop, photos, backup, browse, wi-fi direct, wifi direct, hotspot, offline, nearby, bluetooth
platforms: desktop, android
---
## Send a file or a folder

- **On the computer:** drop files or folders onto a device's page or its card, or anywhere in the window (with several devices it asks which one), use **Send a file** / **Send a folder** on a device's page, or the tray menu.
- **On the phone:** use **Send a file** or **Send a folder** on a computer's page, or share from any app with Android's share sheet. On Android 10 and later each of your devices appears there by name, so a share goes straight to it.
- **From the computer's file manager:** turn it on in Settings → **Send from the file manager**, then right-click files or folders (Finder: Services → Send with my-domain; Windows: Send to; Linux: Open with, Dolphin or Files' Scripts menu). You pick the device in my-domain.

A folder arrives as a folder, with its subfolders, under the other device's downloads. Hidden files and links inside it are skipped, and names that would put a file anywhere else are refused.

A folder, or several files sent at once, shows as **one row** in Transfers: how many files have arrived, the overall progress, speed and time left. **Show files** lists each file inside. Over a direct link up to three of its files go side by side; over the internet relay, one at a time so the rest of your traffic keeps flowing.

Each file is checked when it arrives. If it was damaged on the way, it's sent again by itself (twice at most) before it counts as failed.

## Resume

If the link drops, or an app restarts (either one), an unfinished transfer carries on by itself from where it stopped once both are back and linked, instead of starting over: within 24 hours, and only if the file hasn't changed in the meantime. A folder or batch carries on with the files it hadn't sent yet, and a link being downloaded on a device (**Download on…**) carries on too.

## Transfers

**Transfers** (in the computer's sidebar, and on the phone's Activity tab) lists everything going in and out: progress, speed (smoothed, so it doesn't jump around), time left, **Pause** / **Resume** and **Cancel**. A paused transfer keeps what has arrived and waits, even across restarts, until you resume it, on either device (pausing from the receiving side needs an up-to-date my-domain on the sender). Finished ones stay in the list (the newest 100) with **Open**, **Show in folder** and **Send again**; one sent from this device that failed has **Retry** (for a folder or batch, only the files that didn't go are sent again).

## Big files nearby: Wi-Fi Direct and the phone's hotspot

A file over 20 MB with one of your devices that's near you but not on the same network (or with no network at all) can take a faster route of its own. The transfer starts the usual way and moves over by itself; if anything goes wrong, it simply carries on the usual way. The row in **Transfers** says **via Wi-Fi Direct** or **via phone hotspot**, and so does the map of how your devices are connected.

- **Phone to phone, and phone to a Windows PC:** Wi-Fi Direct. Nobody leaves their Wi-Fi. On the phone it's **Use Wi-Fi Direct for big files when nearby** in Settings (on by default); on a Windows PC, the same switch in Settings.
- **Phone to a Mac or a Linux computer** (or a Windows PC without Wi-Fi Direct): the phone starts its own hotspot, the computer joins it for the transfer and then goes back to the Wi-Fi it was on. Settings → **Use the phone's hotspot for big files** on the computer: **Ask each time** (the default: "Join <phone>'s hotspot for this transfer?"), **Always** or **Never**.
- **With no internet at all** (no shared Wi-Fi, mobile data off, no server): if the two devices are linked over Bluetooth, the offer goes over Bluetooth, encrypted like every message, and the file goes over Wi-Fi Direct or the hotspot. On the computer, the device shows **nearby, offline** when you pick where to send.
- The first time it's needed the phone explains and asks for the permission Android wants for it (called location or nearby devices; your location is never read). If you say no, big files just go the usual way.
- The new connection is checked with the same device keys as any other, and the hotspot's name and passphrase only ever go, encrypted, to your own device.
- A Mac has no Wi-Fi Direct, and Linux doesn't use it here: those use the phone's hotspot. On a Mac, going back to your Wi-Fi uses the password the Mac already has saved.

## Download on another device

Found a download link on one device but want the file on another? Have the other device download it itself, straight from the web, with no copying and pasting:

- **On the phone:** share the link from any app and pick **Download on…** in the share sheet (or **Download on…** in my-domain's own share dialog), then the device.
- **On the computer:** copy the link, then use **Download copied link on…** in the tray menu or the command palette (Ctrl+K / Cmd+K), or the device's own **Download copied link on <device>** command. A saved web link (a `.url` or `.webloc` file) sent with **Send with my-domain** can be downloaded on a device too.

Only devices with an up-to-date my-domain are offered. The device that downloads says how it's going, and so does the one that asked: both show it in **Transfers** ("Downloading report.zip from example.com · requested by Pixel"), with **Cancel** on either side. One download runs at a time; others wait in the queue. If the connection drops, it carries on from where it stopped. The file goes into that device's downloads folder (on a phone, Downloads/my-domain) under the name the site gives it, and is **never opened by itself**.

On the device that downloads, Settings → **Downloads from my devices**:

- **When a device asks:** **Ask the first time for each device** (the default: it shows the site, the file's name and its size when known, with **Allow**, **Always allow <device>** and **Don't allow**), **Always download** or **Never download**. **Forget** clears the devices you always allowed.
- **Biggest download:** a bigger file is refused (100 MB to 4 GB, or no limit; 4 GB to start with).
- **Allow links to the local network:** off by default, so a link to the device itself, your router or anything else at home (192.168…, 10…, `.local` names) is refused, even after a redirect.

Only web links (http and https) are downloaded, and only public ones: nothing signs in and no cookies or passwords are sent, so a link that needs you to be logged in (a private cloud file, a mail attachment) may fail. These settings are part of the computer's settings backup.

## Browse and photos

A device can let your others browse its shared folders and download from them. A computer can also show the phone's recent photos and videos and download them. On the phone, **Photo backup** in Settings copies new photos to a computer you choose, on Wi-Fi only (and only while charging, if you like). These need the phone's photo permission.

## Print a document on a computer's printer

From any app on the phone, **Print** → **my-domain** sends the document to one of your computers as a PDF and prints it there. See [Print from your phone](help:print).

## Limits

- Files move fastest when both devices are on the same network. Through the internet it works, but more slowly.
- A very large folder is capped (10,000 files). On the phone, a folder is copied first, so it needs free space.
