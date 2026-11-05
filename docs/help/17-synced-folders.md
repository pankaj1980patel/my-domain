---
title: Synced folders
keywords: sync, synced, synchronise, synchronize, mirror, folder, notes, documents, two-way, both ways, conflict, trash, pause
platforms: desktop, android
---
A synced folder is kept the same on two of your devices, for example **Documents/Notes** on your computer and a folder on your phone. Change, add or delete a file on one and the other follows. The files go straight between your devices, encrypted like everything else: nothing is stored on a server or in a cloud.

## Start one

- **On the computer:** open the other device's page, find **Synced folders**, choose **Sync a folder…**, pick the folder and how it should sync, then **Sync**.
- **On the other device:** the invitation shows on the computer's page (or the phone's **Synced folders** in Settings). Choose **Accept** and where the folder goes there. Files already in it are merged, never overwritten.

How it syncs:

- **Both ways:** changes on either device go to the other.
- **Only send from here:** this device's changes go to the other; the other's changes stay there.
- **Only receive here:** the other way round.

On the phone, a synced folder lives in **Downloads/my-domain/Sync/<name>** (on Android 10 and older, in the app's own storage). Files other apps put in that folder may not be readable by my-domain; those are skipped.

## When it syncs

About every 30 seconds while both devices are connected to each other, and right away when you choose **Sync now**. It never wakes a sleeping phone just to sync. Big files resume where they stopped if the connection drops.

## Deleted files and conflicts

- A file deleted on one device is moved to the hidden **.mydomain-trash** folder inside the synced folder on the other, and kept there for 7 days.
- If a file was changed on both devices before they synced, both versions are kept: the one changed last keeps the name, the other is saved next to it as **name (conflict from <device> <date>)**, on both devices. Open both, keep what you want, delete the other.
- A file renamed on one device is deleted and added on the other.

## What isn’t synced

Hidden and system files (names starting with a dot, **.DS_Store**, **Thumbs.db**, **desktop.ini**), Office lock files (**~$…**), unfinished downloads (**.part**), links, and files over the size limit (1 GB unless you change it in Settings → **Synced folders**). A folder holds at most 20,000 files.

## Pause and remove

Settings → **Synced folders** lists every synced folder with how it stands: in sync, syncing, conflicts, paused, or a problem. **Pause** stops it on this device until you resume it. **Remove** stops syncing on both devices; the files stay where they are on both.

Both devices need an up-to-date my-domain: an older app can’t sync folders, and is never sent anything about them.
