// What the extension says to my-domain, built and checked here (pure: no
// browser APIs, so `node lib/messages.test.mjs` runs it). The desktop app's
// ext_bridge.rs checks everything again; the limits match it.

/** The native messaging host's name (ext_bridge.rs HOST_NAME). */
export const HOST_NAME = "com.landiscover.mydomain";
/** Bytes of a file per message (ext_bridge.rs MAX_CHUNK: 524288). */
export const MAX_CHUNK = 524288;
/** The biggest file sent (ext_bridge.rs MAX_UPLOAD). */
export const MAX_UPLOAD = 25 * 1024 * 1024;
/** The longest text (ext_bridge.rs MAX_TEXT). */
export const MAX_TEXT = 64 * 1024;
/** The longest link (ext_bridge.rs MAX_URL). */
export const MAX_URL = 4096;
const MAX_TITLE = 300;

/** The menu's actions, in order. */
export const ACTIONS = ["send_page", "send_link", "send_text", "send_image", "download_on", "open_on"];

/** A link that may be sent: http or https, trimmed; else null. */
export function webUrl(url) {
  if (typeof url !== "string") return null;
  const u = url.trim();
  if (!u || u.length > MAX_URL || /[\s\u0000-\u001f\u007f]/.test(u)) return null;
  let parsed;
  try {
    parsed = new URL(u);
  } catch {
    return null;
  }
  if ((parsed.protocol !== "http:" && parsed.protocol !== "https:") || !parsed.hostname) return null;
  return u;
}

/** A tidy title, or undefined. */
export function cleanTitle(title) {
  if (typeof title !== "string") return undefined;
  const t = title.replace(/[\u0000-\u001f\u007f]/g, " ").split(/\s+/).filter(Boolean).join(" ");
  return t ? [...t].slice(0, MAX_TITLE).join("") : undefined;
}

function need(cond, why) {
  if (!cond) throw new Error(why);
}

function deviceOk(id) {
  return typeof id === "string" && /^[A-Za-z0-9_.:-]{1,128}$/.test(id);
}

export function hello() {
  return { type: "hello" };
}

export function listDevices() {
  return { type: "list_devices" };
}

/** A page or a link, as a message on the device. */
export function sendUrl(device, url, title) {
  need(deviceOk(device), "bad_device");
  const u = webUrl(url);
  need(u, "bad_url");
  const msg = { type: "send_url", device, url: u };
  const t = cleanTitle(title);
  if (t) msg.title = t;
  return msg;
}

/** Text onto the device's clipboard. */
export function sendText(device, text) {
  need(deviceOk(device), "bad_device");
  need(typeof text === "string" && text.trim() !== "", "no_text");
  need(new TextEncoder().encode(text).length <= MAX_TEXT, "too_long");
  return { type: "send_text", device, text };
}

export function openUrl(device, url) {
  need(deviceOk(device), "bad_device");
  const u = webUrl(url);
  need(u, "bad_url");
  return { type: "open_url", device, url: u };
}

export function downloadOn(device, url) {
  need(deviceOk(device), "bad_device");
  const u = webUrl(url);
  need(u, "bad_url");
  return { type: "download_on", device, url: u };
}

export function readLater(url, title) {
  const u = webUrl(url);
  need(u, "bad_url");
  const msg = { type: "read_later_add", url: u };
  const t = cleanTitle(title);
  if (t) msg.title = t;
  return msg;
}

/** Bytes to base64 (no Buffer: the same in a browser and node). */
export function toBase64(bytes) {
  let s = "";
  for (let i = 0; i < bytes.length; i += 0x8000) {
    s += String.fromCharCode.apply(null, bytes.subarray(i, i + 0x8000));
  }
  return btoa(s);
}

/** A fresh upload id. */
export function uploadId() {
  const b = new Uint8Array(12);
  crypto.getRandomValues(b);
  return toBase64(b).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
}

/** A file's messages, in order: pieces of MAX_CHUNK, the last one `done`. */
export function fileMessages(device, name, bytes, id = uploadId()) {
  need(deviceOk(device), "bad_device");
  need(bytes instanceof Uint8Array && bytes.length > 0, "empty");
  need(bytes.length <= MAX_UPLOAD, "too_big");
  const out = [];
  for (let offset = 0; offset < bytes.length; offset += MAX_CHUNK) {
    const piece = bytes.subarray(offset, offset + MAX_CHUNK);
    out.push({
      type: "send_file",
      device,
      upload: id,
      name,
      offset,
      data: toBase64(piece),
      done: offset + MAX_CHUNK >= bytes.length,
    });
  }
  return out;
}

const EXT = {
  "image/jpeg": "jpg",
  "image/png": "png",
  "image/gif": "gif",
  "image/webp": "webp",
  "image/avif": "avif",
  "image/svg+xml": "svg",
  "image/bmp": "bmp",
};

/** A file name for a picture from its link and type. */
export function imageName(url, mime) {
  let base = "";
  try {
    const u = new URL(url);
    if (u.protocol !== "data:") base = decodeURIComponent(u.pathname.split("/").pop() || "");
  } catch {
    /* no name in it */
  }
  base = base.replace(/[\\/:*?"<>|\u0000-\u001f]/g, "_").replace(/^\.+/, "").slice(0, 100);
  const ext = EXT[(mime || "").split(";")[0].trim().toLowerCase()];
  if (!base) base = "image";
  if (ext && !/\.[A-Za-z0-9]{2,5}$/.test(base)) base += "." + ext;
  return base;
}

/** A menu item id for an action on a device, and back. */
export function menuId(action, node) {
  return `${action}|${node}`;
}

export function parseMenuId(id) {
  const s = String(id);
  const bar = s.indexOf("|");
  if (bar < 0) return null;
  const action = s.slice(0, bar);
  const node = s.slice(bar + 1);
  if (!ACTIONS.includes(action) || !deviceOk(node)) return null;
  return { action, node };
}

/**
 * The message for a menu click (`info` as contextMenus gives it, `tab`
 * the page), or null when it needs a fetched file (send_image).
 */
export function forMenu(action, node, info, tab) {
  const page = info.pageUrl || (tab && tab.url);
  switch (action) {
    case "send_page":
      return sendUrl(node, page, tab && tab.title);
    case "send_link":
      return sendUrl(node, info.linkUrl, info.linkText || info.selectionText);
    case "send_text":
      return sendText(node, info.selectionText || "");
    case "download_on":
      return downloadOn(node, info.srcUrl || info.linkUrl || page);
    case "open_on":
      return openUrl(node, info.linkUrl || page);
    default:
      return null;
  }
}

/** The _locales key for an answer's error code. */
export function errorKey(code) {
  switch (code) {
    case "app_not_running":
      return "errAppNotRunning";
    case "host_missing":
      return "errHostMissing";
    case "unknown_device":
      return "errUnknownDevice";
    case "no_remote_download":
      return "errNoRemoteDownload";
    case "too_big":
      return "errTooBig";
    case "bad_url":
      return "errBadUrl";
    case "bad_request":
    case "bad_device":
    case "no_text":
    case "too_long":
    case "empty":
      return "errBadRequest";
    default:
      return "errFailed";
  }
}

/** A device's icon group, for drawing (the app's icon names). */
export function iconGroup(icon) {
  switch (icon) {
    case "android_phone":
    case "android_foldable":
    case "phone":
      return "phone";
    case "android_tablet":
    case "tablet":
      return "tablet";
    case "mac_laptop":
    case "win_laptop":
    case "linux_laptop":
    case "laptop":
      return "laptop";
    case "imac":
    case "mac_desktop":
    case "win_desktop":
    case "linux_desktop":
    case "desktop":
      return "desktop";
    case "tv":
      return "tv";
    case "watch":
      return "watch";
    default:
      return "unknown";
  }
}
