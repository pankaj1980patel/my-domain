---
title: Phone audio on the computer, and the other way round
keywords: audio, sound, music, speakers, headphones, play, stream, podcast, video, microphone, capture, computer audio, opus, bluetooth, offline, mute, now playing, media controls, pause, next, previous, skip, seek, volume, spotify
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
- The phone keeps playing too, unless you turn on **Mute this phone while it plays** (see **Mute here** below).
- It sounds best on the same network. With no network at all it can go over Bluetooth, at a lower quality (see **Over Bluetooth** below).
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
- Through the internet the sound is packed with Opus, the codec of video calls: about a tenth of the data, at full quality (48 kHz stereo). Each packet also carries the one before it, so a single lost packet is recovered; a longer gap is filled by Opus itself, which sounds far more natural than a repeat. With a device whose app is older, my-domain falls back to the smaller format it understands.
- **Details** under the audio view shows how much is buffered, gaps filled in, lost packets recovered (and rebuilt by Opus) and dropouts. Many dropouts usually mean weak Wi-Fi: move closer to the router, or use the same network for both devices.
- On the phone, the sound keeps playing with the screen off; my-domain keeps Wi-Fi awake only while it plays.

## Over Bluetooth

When the phone and the computer have no network link at all (no shared Wi-Fi, no internet), the sound can go over Bluetooth instead, as long as both apps are up to date and Bluetooth is on. The audio view then shows **via Bluetooth**.

- Bluetooth carries far less, so the sound is packed smaller (about 32 kbit/s) and waits a little longer before playing (about 150 ms). It sounds fine for music and podcasts, but not as clear as over Wi-Fi.
- **Send audio over** in the audio view chooses, for each device:
  - **Auto** (the default): Wi-Fi or the internet when there's a link, Bluetooth when there isn't. If Wi-Fi comes back, the sound moves to it by itself.
  - **Wi-Fi or internet only**: never Bluetooth.
  - **Bluetooth only**: Bluetooth even when there's a network link (for example, to keep Wi-Fi free). The computer then keeps its Bluetooth connection to the phone open while the sound plays.
- Bluetooth works between a phone and a computer (the computer connects to the phone). Keep them within a few metres.

## Mute here

While this device's sound plays on the other one, you can mute it here, so the sound comes out of the other device only:

- **On the phone:** **Mute this phone while it plays on** the computer, on the computer's page. The phone's media sound is muted while it plays there; the computer still gets every sound, because Android captures it before the volume.
- **On the computer:** **Mute this computer while it plays on** the phone, on the phone's page. On a Mac the sound output is muted (the capture takes the sound before the speakers). On Linux the default output is muted; if that silences what the phone hears too (some PulseAudio setups), my-domain notices within a few seconds, turns the sound back on and says so.
- **Windows:** not offered. Windows records its sound after the speaker mute, so the phone would go quiet too.

The switch is remembered for each device. The sound comes back as it was when the stream stops, when the link drops, and, if my-domain was closed suddenly, the next time it starts. If the device was muted before, or you unmuted it yourself meanwhile, my-domain leaves it alone.

## Now playing and media controls

Each device's page has a **Now playing** tile: what plays there (the artwork, title, artist and app), with **previous**, **play/pause** and **next**, a bar to jump to any point in the track, and its volume. When several apps play at once (Spotify and a podcast, say), choose which one to show and control.

- **On the computer:** open the phone's page. The phone shows what any app with media controls plays (Spotify, YouTube, podcasts and so on); it needs **Notification access**, the same as phone notifications.
- **On the phone:** open the computer's page. A Mac shows and controls Spotify and Music (macOS asks once whether my-domain may control them); Linux any player that supports MPRIS (most do; needs `playerctl`). Windows isn't supported yet.
- On the computer, the command palette also has **Play or pause**, **Next track** and **Previous track** on each device that takes them.

While the tile is open, the device keeps you posted as the song changes; the artwork is sent once per song, and changes come at most about once a second. Nothing is sent while nobody is looking, and the controls only work while the two are connected, so a pause never arrives minutes late. Only your own devices can control what plays. A device whose app is older shows what plays, without the controls, until you update it.
