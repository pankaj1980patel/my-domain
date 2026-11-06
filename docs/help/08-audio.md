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

## Nearby mode

When the phone and the computer are on the same network (or in Bluetooth range) while audio plays, the sound moves by itself to a direct link on your local network, without a break, and waits less before playing. The audio view then shows **Nearby · low latency** and how long the sound takes to arrive, for example "12 ms (was 140 ms)". The connection map shows it on the line too.

- If that link drops, the sound carries on through the internet straight away.
- Turn it off in Settings with **Use nearby mode for media** (on by default; kept in the settings backup).

## When the sound breaks up

my-domain works hard to keep the sound steady:

- It waits a little longer after a dropout and less once things are steady. On the same network it can wait as little as 30 ms; through the internet up to 200 ms.
- A short gap is filled with a soft repeat of what just played, faded out, instead of a click or silence.
- Through the internet the sound is packed smaller, and each packet also carries the one before it, so a single lost packet is recovered.
- **Details** under the audio view shows how much is buffered, gaps filled in, lost packets recovered and dropouts. Many dropouts usually mean weak Wi-Fi: move closer to the router, or use the same network for both devices.
- On the phone, the sound keeps playing with the screen off; my-domain keeps Wi-Fi awake only while it plays.
