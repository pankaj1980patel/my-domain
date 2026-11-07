---
title: Phone screen on the computer, computer screen on the phone
keywords: screen, share, mirror, view, control, remote, touchpad, mouse, keyboard, tap, swipe, screen recording, accessibility, slides, presentation, camera, webcam, document camera, photo, torch, flashlight, insert, scan, scanner, pdf, continuity camera
platforms: desktop, android
---
## The phone's screen on the computer

On the phone's page on the computer, the **Phone screen** tile shows the phone's screen. You can also start it from the phone with **Share this screen** on the computer's page.

- Android asks you **every time** before anything is shared. Nothing is sent without that yes.
- The computer may see the phone's screen only if the phone allows it: sharing from the computer's page counts as once; **Allow <computer> to see this screen** on that page makes it always. When the computer asks on its own and isn't allowed, the phone asks: **Allow once**, **Always** or **Don't allow**, then Android's own prompt.
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

## Insert from phone: a photo or a scan

On the computer, **Take a photo with …** or **Scan documents with …** asks your phone for a picture, like a document scanner in your pocket. Start it from the phone's page (the **Insert from phone** tile), the tray menu, the command palette, or a keyboard shortcut you set in Settings.

- The phone opens its camera **only when you act on it**: if my-domain is open on the phone, a screen asks you to open the camera; otherwise a notification "Your computer wants a photo" opens it when you tap it.
- **Photo:** take it, and it's sent.
- **Scan:** take one or more pages. Each page's edges are found for you; drag the corners to fix them. Pick Original, Grayscale or Black & white, move pages up or down or delete them, then send: the pages become one PDF (A4, or Letter in the US and Canada).
- On the computer it goes to **Downloads** and onto the **clipboard**: a photo as a picture, a scan as a file (or its path where the clipboard can't hold files). A notice says "Scan from … saved", with **Show in folder**.
- **Insert where I am** (Settings, off by default) pastes it straight into the app you're using. It needs the same permission as the phone's touchpad (Accessibility on a Mac).
- Cancel on either side. If nothing comes back in 3 minutes, the request ends by itself. Both apps must be up to date.

## The computer's screen on the phone

The phone's full-screen touchpad shows the computer's screen behind it, so you can see what the pointer does. With **Point & click**, a tap puts the pointer where you touched the picture and clicks there.

- The computer must allow **this phone**: on the phone's page on the computer, under **Permissions**, turn on **Allow <phone> to see this screen** (and **Allow <phone> to control this computer** to be controlled). Both are off until you do, for each phone. If you haven't, the phone says so ("Mac hasn't allowed control from this phone"), and the computer asks you there: **Allow once**, **Always** or **Don't allow**.
- **macOS** needs Screen Recording permission for my-domain, and Accessibility to be controlled.
- **Linux on X11** needs ImageMagick (its `import` tool) to show the screen; control works out of the box.
- **Linux on Wayland** (GNOME, KDE Plasma, Sway...) goes through your desktop's portals, which ask you once.
- **Wayland, seeing the screen:** the screen-cast portal plus GStreamer's PipeWire plugin give about two pictures a second. The first time, your desktop asks which screen to share; my-domain remembers the answer (the portal's restore token), so it doesn't ask again until you revoke it. Without the plugin, `grim` (Sway, Hyprland) or the screenshot portal is used: one picture every two seconds.
- **Wayland, control (touchpad, keyboard, Point & click):** the remote-desktop portal (GNOME, KDE), asked once and remembered. Where there's none (Sway and other wlroots desktops), my-domain uses `/dev/uinput` if you've allowed it (see [Troubleshooting](help:troubleshooting)); there typing is limited to what a US keyboard types and Point & click isn't available.
- **Wayland:** stopping the sharing from your desktop's own indicator stops it here too; watching again asks again after a short pause.
- The health check lists what's present and what to install on Wayland.
- The computer shows a notice when a device starts watching.

## Touchpad and remote

The phone can be a touchpad and keyboard for the computer (move, click, scroll, type, swipe gestures) and a slide remote (next, previous, black screen).

## Limits

The camera sends about a dozen pictures a second, enough to look around; it is not a video call. The phone's screen is a picture about twice a second, not smooth video: fine for checking something or helping someone, not for watching films or games. It works best on the same network.

## Nearby mode

While you watch a screen or the phone's camera and both devices are on the same network (or in Bluetooth range), the picture moves by itself to a direct link on your local network, without a break, and only the newest picture is sent, so it lags less. The view shows **Nearby · low latency** and how long a picture takes to arrive. If that link drops, it carries on through the internet straight away. Turn it off in Settings with **Use nearby mode for media**.
