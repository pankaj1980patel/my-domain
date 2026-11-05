---
title: Phone notifications on the computer
keywords: notification, notifications, mirror, history, mute, apps, reply, dismiss, do not disturb, dnd, quiet, pop-up, alerts, computer notifications, linux, from computer
platforms: desktop, android
---
Your phone's notifications can show on your computer, where you can read them, reply, tap their buttons or dismiss them.

## What it needs

On the phone, allow **Notification access** for my-domain (Settings → Set up this phone). Android asks you in its own settings screen; my-domain can't turn it on itself.

## History

The phone's page on the computer keeps its notifications: the newest 500, grouped by day, with search and a chip for each app. **Clear history** empties it.

- Dismissing one on the computer marks it dismissed.
- When you swipe one away or open it on the phone, it leaves the computer's list too.

## Mute an app

**Mute** an app in the history and its notifications stop popping up on the computer and aren't kept. Unmute it from the same list.

## Follow the phone's Do Not Disturb

Turn on **Stay quiet while my phone is on Do Not Disturb** in the computer's Settings (off by default). While the phone is on Do Not Disturb, the computer holds my-domain's pop-ups and tells you how many came when it ends. They are still kept in the history. Important ones (a ring, find my phone, lost mode, screen sharing, permission questions) always show.

The computer only reads the phone's Do Not Disturb. It never changes it.

## The computer's notifications on the phone

It works the other way too, on Linux: turn on **Show this computer's notifications on my phone** in the computer's Settings (off by default). Then on the phone, open the computer's page and turn on **Show its notifications here**.

- They show on the phone under **Computer notifications**, grouped by computer, saying which computer and app each came from.
- Each app is listed in the computer's Settings as it shows its first notification. Turn an app off there and its notifications stay on the computer.
- Swipe one away on the phone and it closes on the computer too, if it's still there. When it closes on the computer, it leaves the phone.
- my-domain's own notifications (the phone's, shown on the computer) never go back to the phone.

On Windows this isn't available yet: Windows only lets packaged apps read notifications, and my-domain isn't packaged yet. On macOS it isn't possible: macOS doesn't let apps read other apps' notifications.

## Limits

Apps that hide their content on the lock screen, or don't post normal notifications, may show less on the computer.
