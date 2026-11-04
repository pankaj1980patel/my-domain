---
title: Phone audio on the computer, and the other way round
keywords: audio, sound, music, speakers, headphones, play, stream, podcast, video, microphone, capture, computer audio
platforms: desktop, android
---
Play your phone's music, videos and podcasts through the computer's speakers or headphones, over the same encrypted link.

## How to start

- On the computer: the phone's page has a **Phone audio** tile. Press **Play this phone's audio here**.
- Or on the phone: the computer's page has a **Phone audio** tile with **Play on** that computer.

Either way, the phone asks first, and then Android asks every time before it captures any sound. On the computer you can choose which speakers to use and the volume.

## What it needs

- Android 10 or later.
- The microphone permission on the phone. Android requires it to capture what the phone plays; it is only used while you share the audio, and my-domain never records your voice.

## Limits

- **Calls are never sent.** Call audio stays on the phone. For calls, see [Calls on this computer](help:calls).
- Some apps don't allow their sound to be captured; they stay silent on the computer.
- The phone keeps playing too. Turn its volume down if you like.
- It sounds best on the same network. Through the internet the quality is lowered to keep up. It doesn't work over Bluetooth-only links.
- There is a short delay, so it isn't suited to games or lip-synced video.

## The other way round: the computer's sound on the phone

Listen to everything the computer plays through the phone and its earbuds.

- On the phone: the computer's page has a **Play this computer's sound here** tile.
- Or on the computer: the phone's page has **Play on** that phone. The phone asks first and plays nothing until you tap **Play**.

Each time, the computer shows a notification that its sound is playing on your phone, and a **Stop** stays in the tray and the sidebar while it plays.

- **Mac:** macOS 13 or later. macOS asks for the Screen Recording permission the first time (it covers system sound); you may need to restart my-domain after allowing it. The Mac can play a phone's audio and send its own sound at the same time.
- **Linux:** needs PulseAudio or PipeWire's Pulse support. Only one direction at a time, so the sound doesn't loop back.
- **Windows:** only one direction at a time, for the same reason.
