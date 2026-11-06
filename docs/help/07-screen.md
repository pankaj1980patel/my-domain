---
title: Phone screen on the computer, computer screen on the phone
keywords: screen, share, mirror, view, control, remote, touchpad, mouse, keyboard, tap, swipe, screen recording, accessibility, slides, presentation, camera, webcam, document camera, photo, torch, flashlight
platforms: desktop, android
---
## The phone's screen on the computer

On the phone's page on the computer, the **Phone screen** tile shows the phone's screen. You can also start it from the phone with **Share this screen** on the computer's page.

- Android asks you **every time** before anything is shared. Nothing is sent without that yes.
- The phone shows a notification while it shares, with **Stop**, and **Allow control** or **View only** (view only at each start).
- It stops when you press Stop on either side, when the computer stops watching, or when the phone's screen goes off or locks. The lock screen is never sent.

**Control from the computer:** with Allow control on the phone and its Accessibility helper turned on, a click is a tap, a held click a long press, and a drag a swipe. Back, Home and Recent apps buttons are there too.

## The phone's camera on the computer

On the phone's page on the computer, the **Phone camera** tile shows what the phone's camera sees: something across the room, or a page on the desk (the phone as a document camera). Press **Ask to see its camera**.

- The phone asks **every time**: "Show your camera to …?" Nothing is shown until you tap **Allow** on the phone. The first time, Android also asks for the camera permission; my-domain says why first.
- While it shows, the phone has a notification "Your camera is on for …" with **Stop**.
- From the computer you can switch between the back and front cameras, turn the torch on and off, go full screen, and **Take photo**: a full-size picture that arrives in Downloads (the notice has **Show in folder**).
- It stops when you press Stop on either side, when the computer's link goes, and when the phone locks or its screen goes off. To keep it going on a locked phone, turn on **Keep streaming while locked** in the phone's Settings (off by default).
- The pictures are never kept, on the phone or the computer, unless you take a photo. Both apps must be up to date: an older phone doesn't show the tile.

## The computer's screen on the phone

The phone's full-screen touchpad shows the computer's screen behind it, so you can see what the pointer does. With **Point & click**, a tap puts the pointer where you touched the picture and clicks there.

- The computer must allow it: **Let my phone see this screen** in its Settings (and **Let my phone control this computer** to be controlled).
- **macOS** needs Screen Recording permission for my-domain, and Accessibility to be controlled.
- **Linux** needs a screenshot tool (grim on Wayland, ImageMagick on X11); control works on X11 or XWayland.
- The computer shows a notice when a device starts watching.

## Touchpad and remote

The phone can be a touchpad and keyboard for the computer (move, click, scroll, type, swipe gestures) and a slide remote (next, previous, black screen).

## Limits

The camera sends about a dozen pictures a second, enough to look around; it is not a video call. The phone's screen is a picture about twice a second, not smooth video: fine for checking something or helping someone, not for watching films or games. It works best on the same network.
