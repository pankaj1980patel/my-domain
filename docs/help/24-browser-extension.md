---
title: Browser extension
keywords: browser, extension, add-on, addon, chrome, edge, brave, chromium, firefox, right-click, context menu, send page, send link, send text, send image, picture, open on, download on, read later, native messaging, toolbar
platforms: desktop
---
The **my-domain browser extension** works in Chrome, Edge, Brave, Chromium and Firefox. Right-click a page, a link, some selected text or a picture and send it straight to one of your devices, or use its toolbar button.

- **Send page to** and **Send link to** a device: the link arrives as a message.
- **Send selected text to** a device: the text goes onto its clipboard.
- **Send image to** a device: the picture arrives as a file (up to 25 MB). If the browser can't fetch it, its link is sent instead.
- **Download on** a device: that device downloads the link itself (see [Files](help:files)); only devices whose app can do it are listed.
- **Open on** a device: the page or link opens in that device's browser.
- **Read later**: saves the page or link to your [Read later](help:read-later) list on all your devices.

The toolbar button shows your devices with their icons and whether each is online, with **Send page**, **Open there** and **Download there** for the page you're on, and **Read later**.

The extension talks only to the my-domain app on this computer, which must be running and signed in. It never talks to the internet itself, and it can reach only your own devices.

## Set it up

1. **Let the browsers reach my-domain.** In my-domain, open **Settings** > **Browser extension** and click **Install**. Each browser found on this computer is listed with **Ready** once it's set up. Restart a browser that was open.
2. **Add the extension to the browser.** Until it's in a store, load it from the `apps/browser-extension` folder of my-domain's source:
   - **Chrome, Edge, Brave, Chromium:** open `chrome://extensions` (`edge://extensions` in Edge), turn on **Developer mode**, click **Load unpacked** and choose the `apps/browser-extension` folder. Its id is always `jonpnfgijnakjfneooofcaeobabdecpj`, which my-domain allows.
   - **Firefox:** open `about:debugging#/runtime/this-firefox`, click **Load Temporary Add-on…** and choose `apps/browser-extension/manifest.json`. Firefox removes a temporary add-on when it closes; a signed copy stays.
3. Click the extension's toolbar button. Your devices should be listed. To send pictures from any site, click **Allow sending pictures from any site** once.

### Installed from a store

A store gives the extension its own id. Copy it from the browser's extensions page, paste it into **Store extension ids** in **Settings** > **Browser extension** and click **Install again**. Firefox always uses `my-domain@landiscover.app`.

### Remove it

Click **Remove** in **Settings** > **Browser extension** to take my-domain out of every browser, then remove the extension from the browser's extensions page.

## If it doesn't work

- **"This browser can't reach my-domain yet"**: click **Install** in **Settings** > **Browser extension** and restart the browser. If you moved the my-domain app, click **Install again** so the browsers find it where it is now.
- **"my-domain isn't running on this computer"**: open my-domain and sign in. It can keep running in the tray or menu bar with its window closed.
- **No devices**: add your devices in my-domain first; the list updates every few minutes, or click the refresh button in the toolbar popup.
- **Download on isn't offered for a device**: that device's app is too old to download links; update it.

## What's set up where

my-domain writes a small "native messaging host" file that tells each browser where the my-domain app is and which extension may start it:

- **macOS:** `~/Library/Application Support/Google/Chrome/NativeMessagingHosts`, `…/Microsoft Edge/…`, `…/BraveSoftware/Brave-Browser/…`, `…/Chromium/…` and `…/Mozilla/NativeMessagingHosts`.
- **Linux:** `~/.config/google-chrome/NativeMessagingHosts`, `~/.config/microsoft-edge/…`, `~/.config/BraveSoftware/Brave-Browser/…`, `~/.config/chromium/…` and `~/.mozilla/native-messaging-hosts`.
- **Windows:** the files are kept with my-domain's settings and registered under `HKEY_CURRENT_USER\Software\…\NativeMessagingHosts` for each browser.

The file is named `com.landiscover.mydomain.json`. The browser starts my-domain's own program, which passes each request to the running app over a channel only your user account can open, and the app checks every request again before doing anything.
