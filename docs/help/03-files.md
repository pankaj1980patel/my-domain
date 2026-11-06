---
title: Sending files and folders
keywords: file, files, print, printer, folder, send, transfer, transfers, resume, download, share, share sheet, finder, explorer, file manager, drag, drop, photos, backup, browse, wi-fi direct, wifi direct, hotspot, offline, nearby, bluetooth
platforms: desktop, android
---
## Send a file or a folder

- **On the computer:** drop files or folders onto a device's page, use **Send a file** / **Send a folder** on it, or the tray menu.
- **On the phone:** use **Send a file** or **Send a folder** on a computer's page, or share from any app with Android's share sheet. On Android 10 and later each of your devices appears there by name, so a share goes straight to it.
- **From the computer's file manager:** turn it on in Settings → **Send from the file manager**, then right-click files or folders (Finder: Services → Send with my-domain; Windows: Send to; Linux: Open with, Dolphin or Files' Scripts menu). You pick the device in my-domain.

A folder goes file by file with its subfolders. Hidden files and links inside it are skipped.

## Resume

If the link drops, or an app restarts, an unfinished transfer carries on from where it stopped instead of starting over: within 24 hours, and only if the file hasn't changed in the meantime.

## Transfers

**Transfers** (in the computer's sidebar, and on the phone's Activity tab) lists everything going in and out: progress, speed, time left and **Cancel**. Finished ones stay in the list (the newest 100) with **Open**, **Show in folder** and **Send again**.

## Big files nearby: Wi-Fi Direct and the phone's hotspot

A file over 20 MB with one of your devices that's near you but not on the same network (or with no network at all) can take a faster route of its own. The transfer starts the usual way and moves over by itself; if anything goes wrong, it simply carries on the usual way. The row in **Transfers** says **via Wi-Fi Direct** or **via phone hotspot**, and so does the map of how your devices are connected.

- **Phone to phone, and phone to a Windows PC:** Wi-Fi Direct. Nobody leaves their Wi-Fi. On the phone it's **Use Wi-Fi Direct for big files when nearby** in Settings (on by default); on a Windows PC, the same switch in Settings.
- **Phone to a Mac or a Linux computer** (or a Windows PC without Wi-Fi Direct): the phone starts its own hotspot, the computer joins it for the transfer and then goes back to the Wi-Fi it was on. Settings → **Use the phone's hotspot for big files** on the computer: **Ask each time** (the default: "Join <phone>'s hotspot for this transfer?"), **Always** or **Never**.
- **With no internet at all** (no shared Wi-Fi, mobile data off, no server): if the two devices are linked over Bluetooth, the offer goes over Bluetooth, encrypted like every message, and the file goes over Wi-Fi Direct or the hotspot. On the computer, the device shows **nearby, offline** when you pick where to send.
- The first time it's needed the phone explains and asks for the permission Android wants for it (called location or nearby devices; your location is never read). If you say no, big files just go the usual way.
- The new connection is checked with the same device keys as any other, and the hotspot's name and passphrase only ever go, encrypted, to your own device.
- A Mac has no Wi-Fi Direct, and Linux doesn't use it here: those use the phone's hotspot. On a Mac, going back to your Wi-Fi uses the password the Mac already has saved.

## Browse and photos

A device can let your others browse its shared folders and download from them. A computer can also show the phone's recent photos and videos and download them. On the phone, **Photo backup** in Settings copies new photos to a computer you choose, on Wi-Fi only (and only while charging, if you like). These need the phone's photo permission.

## Print a document on a computer's printer

From any app on the phone, **Print** → **my-domain** sends the document to one of your computers as a PDF and prints it there. See [Print from your phone](help:print).

## Limits

- Files move fastest when both devices are on the same network. Through the internet it works, but more slowly.
- A very large folder is capped (10,000 files). On the phone, a folder is copied first, so it needs free space.
