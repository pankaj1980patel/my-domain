---
title: Synced folders
keywords: sync, synced, case, capital letters, accents, accented letters, unicode, normalization, NFC, NFD, uppercase, lowercase, case conflict, synchronise, synchronize, mirror, folder, notes, documents, two-way, both ways, conflict, trash, pause
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
- A file renamed on one device is deleted and added on the other. A rename that only changes capital letters (**Notes.txt** to **notes.txt**) is just a rename on the other device too.

## What isn’t synced

Hidden and system files (names starting with a dot, **.DS_Store**, **Thumbs.db**, **desktop.ini**), Office lock files (**~$…**), unfinished downloads (**.part**), links, and files over the size limit (1 GB unless you change it in Settings → **Synced folders**). A folder holds at most 20,000 files.

## Names that differ only by capital letters

Linux and an Android app’s own storage keep **Notes.txt** and **notes.txt** as two files; a Mac (as it comes), Windows and a phone’s shared storage take them for one. my-domain checks this once for each synced folder, and on a computer or phone that can’t tell them apart it never overwrites or deletes a file because of it:

- A file whose name differs from another file’s there only by capital letters is saved as **name (case conflict from <device>)**. The folder’s line in Settings → **Synced folders** says how many: “2 files couldn’t keep their names on this computer”. They still sync both ways: edit or delete one and the other device follows. Once the other file is gone, it takes its own name again.
- A file in a folder whose name differs only by capital letters (**Docs** and **docs**) goes in the folder as it’s already spelled there, and is counted the same way.
- To fix it for good, rename one of the two files on the device that has both.

## Names with accents

An accented letter such as **é** can be written two ways inside a file name: as one character, or as **e** followed by an accent. They look the same, and a Mac takes them for the same name (and may show a file’s name either way), while Linux, Windows and Android keep them as two different names. my-domain treats them as one name everywhere:

- A file whose name is written one way on one device and the other way on the other is the same file: it isn’t copied again or moved to the trash, and edits and deletes go both ways. Each device keeps the name as it has it.
- If a Linux or Windows computer has two files whose names differ only in how an accent is written, a Mac (or another device that takes them for one) gets the second as **name (case conflict from <device>)**, counted like the files above: neither overwrites the other.

## Pause and remove

Settings → **Synced folders** lists every synced folder with how it stands: in sync, syncing, conflicts, paused, or a problem. **Pause** stops it on this device until you resume it. **Remove** stops syncing on both devices; the files stay where they are on both.

Both devices need an up-to-date my-domain: an older app can’t sync folders, and is never sent anything about them.
