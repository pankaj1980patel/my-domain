---
title: Calls on this computer
keywords: call, calls, phone call, answer, decline, hang up, dial, keypad, bluetooth, hands-free, headset, microphone, speaker, pipewire, ofono, phone link, macos, windows, linux
platforms: desktop, android
---
Your computer can act like a car kit for your phone over Bluetooth: see who is calling, answer, decline, hang up, dial, use the keypad and mute. It is off by default; turn it on in the computer's Settings → **Calls on this computer**.

## Before you start

Pair the phone with the computer in the computer's own Bluetooth settings. my-domain never pairs devices for you, and never reads your contacts.

## What works on each computer

**Mac:** the call controls work. The call's sound stays on the phone, because macOS doesn't let apps carry call audio. You talk and listen on the phone (or the headset connected to it).

**Linux:** the controls work and so does the sound: you can talk through the computer's speaker and microphone, or any you pick, and move the sound back to the phone during a call. If PipeWire (or oFono) is already the phone's hands-free on that computer, my-domain can't take the phone; Settings says so and shows how to turn that off. It doesn't change it for you. Sound is phone-call quality.

**Windows:** the controls work. The call's sound stays on the phone, because Windows only lets system drivers carry call audio. If Phone Link's Calls already holds the phone, Settings explains how to turn it off.

## Limits

- The phone must be in Bluetooth range of the computer.
- This is about calls only. To play music or video from the phone on the computer, see [Phone audio on the computer](help:audio).
- The calls themselves stay on the phone and its mobile plan. my-domain doesn't make internet calls.
