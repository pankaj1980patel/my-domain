// The extension's background: the right-click menu, and every request to
// my-domain (through the native messaging host, the desktop app's own
// executable; see ext_bridge.rs). Chrome runs it as a service worker,
// Firefox as an event page; both as a module.

import * as M from "./lib/messages.js";

const api = globalThis.browser ?? globalThis.chrome;
const t = (key, subs) => api.i18n.getMessage(key, subs) || key;

/** Ask my-domain; always an answer object ({ok, ...} or {ok:false, error}). */
export async function ask(msg) {
  try {
    const reply = await api.runtime.sendNativeMessage(M.HOST_NAME, msg);
    return reply && typeof reply === "object" ? reply : { ok: false, error: "failed" };
  } catch (e) {
    // The browser couldn't start the host: not installed from my-domain's
    // Settings, or this extension's id isn't allowed.
    return { ok: false, error: "host_missing", detail: String(e && e.message ? e.message : e) };
  }
}

function notify(title, message) {
  try {
    api.notifications.create({
      type: "basic",
      iconUrl: api.runtime.getURL("icons/128x128.png"),
      title,
      message: message || "",
    });
  } catch {
    /* notifications off */
  }
}

function errorText(reply) {
  const key = M.errorKey(reply.error);
  const base = t(key);
  return key === "errFailed" && reply.detail ? `${base} ${reply.detail}` : base;
}

// ---------------------------------------------------------------------------
// Devices and the menu
// ---------------------------------------------------------------------------

async function loadDevices() {
  const reply = await ask(M.listDevices());
  const devices = reply.ok && Array.isArray(reply.devices) ? reply.devices : [];
  await api.storage.local.set({ devices, devicesError: reply.ok ? null : reply.error || "failed" });
  return reply;
}

const PARENTS = [
  { id: "send_page", contexts: ["page"], title: "menuSendPage" },
  { id: "send_link", contexts: ["link"], title: "menuSendLink" },
  { id: "send_text", contexts: ["selection"], title: "menuSendText" },
  { id: "send_image", contexts: ["image"], title: "menuSendImage" },
  { id: "download_on", contexts: ["link", "image"], title: "menuDownloadOn" },
  { id: "open_on", contexts: ["page", "link"], title: "menuOpenOn" },
];

let building = Promise.resolve();

/** Rebuild the menu from the saved device list. */
function buildMenu() {
  building = building.then(async () => {
    const { devices = [] } = await api.storage.local.get("devices");
    await api.contextMenus.removeAll();
    for (const p of PARENTS) {
      api.contextMenus.create({ id: p.id, title: t(p.title), contexts: p.contexts });
      const targets = p.id === "download_on" ? devices.filter((d) => d.can_download) : devices;
      if (targets.length === 0) {
        api.contextMenus.create({
          id: `${p.id}|none`,
          parentId: p.id,
          title: t("menuNoDevices"),
          contexts: p.contexts,
          enabled: false,
        });
      }
      for (const d of targets) {
        api.contextMenus.create({
          id: M.menuId(p.id, d.node),
          parentId: p.id,
          title: d.online ? d.name : t("deviceOffline", [d.name]),
          contexts: p.contexts,
        });
      }
    }
    api.contextMenus.create({ id: "read_later", title: t("menuReadLater"), contexts: ["page", "link"] });
  });
  return building;
}

async function refresh() {
  const reply = await loadDevices();
  await buildMenu();
  return reply;
}

api.runtime.onInstalled.addListener(() => {
  refresh();
  api.alarms.create("refresh", { periodInMinutes: 5 });
});
api.runtime.onStartup.addListener(() => {
  refresh();
});
api.alarms.onAlarm.addListener((a) => {
  if (a.name === "refresh") refresh();
});

// ---------------------------------------------------------------------------
// Doing it
// ---------------------------------------------------------------------------

/** Fetch a picture's bytes: here, else inside the page (its cookies). */
async function fetchImage(src, tabId) {
  try {
    const r = await fetch(src, { credentials: "include" });
    if (r.ok) {
      const blob = await r.blob();
      return { bytes: new Uint8Array(await blob.arrayBuffer()), mime: blob.type };
    }
  } catch {
    /* no access from here: try in the page */
  }
  if (tabId === undefined || !api.scripting) return null;
  try {
    const [res] = await api.scripting.executeScript({
      target: { tabId },
      args: [src, M.MAX_UPLOAD],
      func: async (url, max) => {
        const r = await fetch(url);
        if (!r.ok) return null;
        const blob = await r.blob();
        if (blob.size > max) return { tooBig: true };
        const buf = new Uint8Array(await blob.arrayBuffer());
        let s = "";
        for (let i = 0; i < buf.length; i += 0x8000) s += String.fromCharCode.apply(null, buf.subarray(i, i + 0x8000));
        return { b64: btoa(s), mime: blob.type };
      },
    });
    const v = res && res.result;
    if (!v) return null;
    if (v.tooBig) return { tooBig: true };
    const bin = atob(v.b64);
    const bytes = new Uint8Array(bin.length);
    for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i);
    return { bytes, mime: v.mime };
  } catch {
    return null;
  }
}

/** Send a picture as a file; its link as a message if it can't be fetched. */
export async function sendImage(node, src, tabId, pageTitle) {
  const got = await fetchImage(src, tabId);
  if (got && got.tooBig) return { ok: false, error: "too_big" };
  if (!got || !got.bytes || got.bytes.length === 0) {
    const reply = await ask(M.sendUrl(node, src, pageTitle));
    return reply.ok ? { ...reply, linkOnly: true } : reply;
  }
  if (got.bytes.length > M.MAX_UPLOAD) return { ok: false, error: "too_big" };
  let reply = { ok: false, error: "failed" };
  for (const msg of M.fileMessages(node, M.imageName(src, got.mime), got.bytes)) {
    reply = await ask(msg);
    if (!reply.ok) break;
  }
  return reply;
}

/** Run an action (the menu's or the popup's); the answer. */
export async function run(action, node, info, tab) {
  try {
    if (action === "send_image") return await sendImage(node, info.srcUrl, tab && tab.id, tab && tab.title);
    if (action === "read_later") return await ask(M.readLater(info.linkUrl || info.pageUrl || tab.url, info.linkUrl ? info.linkText : tab && tab.title));
    const msg = M.forMenu(action, node, info, tab);
    return msg ? await ask(msg) : { ok: false, error: "bad_request" };
  } catch (e) {
    return { ok: false, error: e && e.message ? e.message : "failed" };
  }
}

function doneText(action, reply, name) {
  if (reply.linkOnly) return t("doneImageLink", [name]);
  switch (action) {
    case "read_later":
      return t("doneReadLater");
    case "open_on":
      return t("doneOpen", [name]);
    case "download_on":
      return t("doneDownload", [name]);
    default:
      return t("doneSent", [name]);
  }
}

async function nameOf(node) {
  const { devices = [] } = await api.storage.local.get("devices");
  return (devices.find((d) => d.node === node) || {}).name || node;
}

api.contextMenus.onClicked.addListener(async (info, tab) => {
  let action;
  let node = "";
  if (info.menuItemId === "read_later") {
    action = "read_later";
  } else {
    const parsed = M.parseMenuId(info.menuItemId);
    if (!parsed) return;
    ({ action, node } = parsed);
  }
  const reply = await run(action, node, info, tab);
  const name = node ? await nameOf(node) : "";
  if (reply.ok) notify(t("extName"), doneText(action, reply, name));
  else notify(t("failedTitle"), errorText(reply));
  if (reply.error === "unknown_device") refresh();
});

// The popup asks through here, so pictures and long sends outlive it.
api.runtime.onMessage.addListener((msg, sender, sendResponse) => {
  if (sender.id !== api.runtime.id || !msg || typeof msg !== "object") return false;
  const answer = async () => {
    if (msg.kind === "refresh") return refresh();
    if (msg.kind === "run") {
      const reply = await run(msg.action, msg.node, msg.info || {}, msg.tab || {});
      return { ...reply, text: reply.ok ? doneText(msg.action, reply, await nameOf(msg.node)) : errorText(reply) };
    }
    return { ok: false, error: "bad_request" };
  };
  answer().then(sendResponse, () => sendResponse({ ok: false, error: "failed" }));
  return true;
});
