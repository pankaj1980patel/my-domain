---
title: Phone notifications on the computer
keywords: notification, notifications, mirror, history, mute, apps, reply, dismiss, buttons, actions, conversation, private mode, otp, bank, locked, in use, do not disturb, dnd, quiet, pop-up, alerts, computer notifications, linux, from computer
platforms: desktop, android
---
Your phone's notifications can show on your computer, where you can read them, reply, tap their buttons or dismiss them.

## What it needs

On the phone, allow **Notification access** for my-domain (Settings → Set up this phone). Android asks you in its own settings screen; my-domain can't turn it on itself.

## Reply, buttons and dismiss

On the phone's page, **Notifications** shows what's on the phone now, one conversation at a time: the app's icon, the sender's picture, the last few messages and any picture in the notification.

- **Reply** right there for any messaging app that takes replies (WhatsApp, Messages, Telegram and the like). The reply goes from the phone, as if you'd typed it there.
- The notification's own buttons (Mark as read, Archive...) are under it; press one and the phone presses it.
- **Dismiss** removes the whole conversation here and on the phone. Swiped away or opened on the phone, it leaves the computer too.

The pop-up on the computer answers too, while the window is in the background:

- **macOS**: a reply field when it takes a reply, else its buttons and **Mute** under Options. **Dismiss** dismisses it on the phone too; clicking it opens the phone's page.
- **Windows**: a reply box, its buttons, **Mute** and **Dismiss** in the toast. Closing it with its X dismisses it on the phone. One that goes away on the phone leaves the screen.
- **Linux**: a plain pop-up; reply and buttons are on the phone's page.

## What doesn't come

Only notifications worth seeing come: not ongoing ones (music, downloads, navigation, a running timer), group summaries, the system's own status or quiet (low-importance) channels. The same notification posted again with nothing new doesn't pop up again; an update that the app marks as quiet updates the list without a pop-up.

## Private mode

**Private mode** (on the phone's Notifications tile, on by default) keeps sensitive notifications on the phone: one-time codes, banks, payment apps and password managers show only "New notification from" the app. Their text never leaves the phone, and nothing of it is kept in the history. You can still dismiss them.

The computer applies the same rule to a phone with an older my-domain app, which sends everything: it drops the content before showing or keeping it.

## Only while the phone isn't in use

Turn on **Only while my phone isn't in use** and notifications come to the computer only while the phone is locked or its screen is off. While you're using the phone, you see them there.

## History

The phone's page on the computer keeps its notifications: the newest 500, grouped by day, with search and a chip for each app. **Clear history** empties it.

- Dismissing one on the computer marks it dismissed.
- When you swipe one away or open it on the phone, it leaves the computer's list too.

## Mute an app

**Mute** an app in the history, on its notification, or from the pop-up, and its notifications stop popping up on the computer and aren't kept. Unmute it from the same list.

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
