---
title: Share mouse and keyboard
keywords: share mouse, share keyboard, mouse and keyboard, kvm, barrier, synergy, input leap, two computers, move the pointer, edge of the screen, screen edge, hotkey, ctrl alt esc, take back, cmd ctrl, control another computer, clipboard follows
platforms: desktop
---
Use one mouse and keyboard on all your computers. Put your laptop next to your desktop, move the pointer off the side of one screen, and it carries on onto the other computer's screen; type, click and scroll there as if the mouse and keyboard were plugged into it. Move back across the same edge and you're home.

It works between your own Macs, Windows PCs and Linux computers that are signed in to your account, with my-domain open on both.

## Set it up

1. On one computer, open **Settings → Share mouse and keyboard**.
2. Your other computers that can share are listed under **Your computers**. Drag each one into the slot for the side of this screen it sits on (**Left**, **Right**, **Above** or **Below**), or pick the side from the list next to its name.
3. That's it: the other computer learns where this one sits, so you can go both ways. One computer per side.

To take a computer off, choose **×** on it, or **Not placed** in the list.

## Use it

- **Go over:** move the pointer off the edge with a computer next to it. The pointer appears at the matching spot on the other screen's facing edge. A screen's corners don't start it, so hot corners keep working.
- **Come back:** move the pointer back across that same edge on the other computer.
- **Come back at once:** press the hotkey on this computer's keyboard: **Ctrl+Alt+Esc** (**Control+Option+Esc** on a Mac) unless you chose another in Settings (**Ctrl+Alt+F12** or **Scroll Lock**).
- **Take back:** on the computer being controlled, a small bar at the top of the screen says so; choose **Take back** there (or **Stop** in Settings on either computer).

If the connection between the two drops, or either computer stops answering for a few seconds, control comes back by itself: your mouse and keyboard are never left stuck on the other computer, and nothing stays held down over there.

## Clipboard

With **Clipboard follows the pointer** on (on both computers), what you copied goes with you to the computer you move to, so you can copy on one and paste on the other. Text only. A password copied from a password manager never goes.

## Cmd and Ctrl

Between a Mac and a Windows or Linux computer, **Swap Cmd and Ctrl** (on by default) makes shortcuts work the way your hands expect: Cmd+C on the Mac's keyboard copies on Windows, and Ctrl+C on a Windows keyboard copies on the Mac. Turn it off to send the keys exactly as pressed.

## Safety

- Only your own computers, signed in to your account, can control each other; everything goes encrypted over your own connection.
- While another computer controls this one, the bar at the top of the screen and the menu bar or tray icon say so.
- To never be controlled, turn off **Let my other computers control this one** on that computer. To stop this computer's mouse and keyboard from going anywhere, turn off **Share this computer's mouse and keyboard**.

## If it doesn't work

- **On a Mac,** my-domain needs **System Settings → Privacy & Security → Accessibility** to capture and move the mouse and keyboard. Allow it there yourself (my-domain never turns it on), then try again.
- **On Linux,** it needs an X11 session. It isn't available on Wayland, which doesn't let apps capture or move the pointer: choose an X11 (Xorg) session at sign-in to use it.
- **Nothing happens at the edge:** check the computer is placed on that side, that it's connected (Settings lists "not connected" otherwise), and that the other computer allows being controlled.
- **Several screens:** only an outer edge works, one with no screen of the same computer beyond it. On the computer being controlled, the pointer comes in on its outermost screen at that side.
- **Windows:** a window running as administrator may not take the other computer's input unless my-domain also runs as administrator (Windows' own rule).
