---
title: Wake a sleeping computer
keywords: wake, wake up, wake-on-lan, wake on lan, wol, magic packet, sleep, asleep, sleeping, turn on, power on, network access, womp, ethtool, bios, uefi
platforms: desktop, android
---
A computer that's asleep can't answer, so my-domain can't reach it. With **Wake it** your phone (or another computer) sends it the standard Wake-on-LAN "magic packet" over your home network, and the computer wakes up and connects again.

## Turn it on

1. On the computer, open **Settings** and turn on **Let my other devices wake this computer** (off by default). From then on it tells your other devices its network card's address and network, and they remember it for when it's asleep.
2. Set the computer itself to wake for network access. my-domain can't do this for you:
   - **Mac:** System Settings → **Battery** (or **Energy Saver**) → **Options** → **Wake for network access**. my-domain's Settings shows whether it's on.
   - **Windows:** Device Manager → your network adapter → **Properties** → **Power Management** → **Allow this device to wake the computer** (and **Only allow a magic packet**). Many PCs also need **Wake on LAN** turned on in the BIOS/UEFI setup.
   - **Linux:** `sudo ethtool -s <card> wol g` (for example `eth0` or `enp3s0`; it may need to be repeated after each restart, or set in NetworkManager), and **Wake on LAN** in the BIOS/UEFI setup.

A wired (Ethernet) connection wakes far more reliably than Wi-Fi: most computers can't be woken over Wi-Fi at all.

## Wake it

Open the computer's page while it's offline and choose **Wake it**. It shows **Waking…** until the computer connects, or for up to a minute.

- **On the phone:** the button is under the computer's name. An automation can do it too: **Wake it** is one of the things an automation can do to a computer.
- **On another computer:** the button is next to **Connect** on its page.

## How it reaches it

The wake-up goes as a broadcast on the network you're on, and on the computer's own network. It only works from the same network (the same home Wi-Fi or wired network), since routers don't pass these packets on.

Away from home? If another of your devices is connected and on the sleeping computer's network (another computer, say), it's asked to send the wake-up too, so you can wake your desktop through your laptop at home. Any of your devices with a recent app does this for the others; there's nothing to turn on.

## If it doesn't wake

- Check the computer's own setting above. On a Mac, **Wake for network access** has to be on.
- Use a wired connection if you can.
- The computer must be asleep, not shut down. Some PCs can be woken from shut down too, if the BIOS/UEFI allows it.
- After the computer's network changes (another router, another network card), open my-domain on it once so your devices learn the new address.
- Turning **Let my other devices wake this computer** off makes your devices forget its address the next time they see it.
