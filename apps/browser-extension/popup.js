// The toolbar popup: this page to Read later, and your devices with what
// to do on each (send the page, open it there, download it there).

import { iconGroup, webUrl } from "./lib/messages.js";

const api = globalThis.browser ?? globalThis.chrome;
const t = (key, subs) => api.i18n.getMessage(key, subs) || key;
const $ = (id) => document.getElementById(id);

// One stroke style, 24x24 (the desktop app's shapes, simplified).
const SHAPES = {
  phone: ["M8 2h8a2 2 0 0 1 2 2v16a2 2 0 0 1-2 2H8a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2z", "M11 18h2"],
  tablet: ["M5 3h14a2 2 0 0 1 2 2v14a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2z", "M11 18h2"],
  laptop: ["M5 5h14v10H5z", "M2 19h20l-2-4H4z"],
  desktop: ["M3 4h18v12H3z", "M9 20h6", "M12 16v4"],
  tv: ["M3 6h18v12H3z", "M8 21h8", "M8 3l4 3 4-3"],
  watch: ["M8 7h8v10H8z", "M9 7l1-4h4l1 4", "M9 17l1 4h4l1-4"],
  unknown: ["M4 5h16v14H4z", "M10 10a2 2 0 1 1 3 1.7c-.6.4-1 .8-1 1.3", "M12 16h.01"],
};

function icon(name) {
  const ns = "http://www.w3.org/2000/svg";
  const svg = document.createElementNS(ns, "svg");
  svg.setAttribute("viewBox", "0 0 24 24");
  svg.setAttribute("aria-hidden", "true");
  svg.classList.add("icon");
  for (const d of SHAPES[iconGroup(name)]) {
    const p = document.createElementNS(ns, "path");
    p.setAttribute("d", d);
    svg.appendChild(p);
  }
  return svg;
}

function localize() {
  for (const el of document.querySelectorAll("[data-i18n]")) el.textContent = t(el.dataset.i18n);
  for (const el of document.querySelectorAll("[data-i18n-title]")) {
    el.title = t(el.dataset.i18nTitle);
    el.setAttribute("aria-label", el.title);
  }
  document.documentElement.lang = api.i18n.getUILanguage();
}

let tab = null;

function say(text, kind) {
  const s = $("status");
  s.textContent = text || "";
  s.className = kind || "muted";
}

async function act(action, node, button) {
  button.disabled = true;
  say(t("working"));
  const reply = await api.runtime.sendMessage({
    kind: "run",
    action,
    node,
    info: { pageUrl: tab.url },
    tab: { id: tab.id, url: tab.url, title: tab.title },
  });
  button.disabled = false;
  say(reply && reply.text, reply && reply.ok ? "ok" : "error");
}

function render(devices, error) {
  const list = $("devices");
  list.replaceChildren();
  const pageOk = tab && webUrl(tab.url);
  $("read-later").disabled = !pageOk;
  if (error) {
    say(t(error === "host_missing" ? "errHostMissing" : error === "app_not_running" ? "errAppNotRunning" : "errFailed"), "error");
    return;
  }
  if (devices.length === 0) {
    say(t("popupNoDevices"));
    return;
  }
  say("");
  for (const d of devices) {
    const li = document.createElement("li");
    li.appendChild(icon(d.icon));
    const who = document.createElement("div");
    who.className = "who";
    const name = document.createElement("strong");
    name.textContent = d.name;
    const sub = document.createElement("span");
    sub.className = "muted small";
    sub.textContent = [d.online ? t("online") : t("offline"), d.model].filter(Boolean).join(" · ");
    const dot = document.createElement("span");
    dot.className = d.online ? "dot on" : "dot";
    name.prepend(dot);
    who.append(name, sub);
    li.appendChild(who);
    const actions = document.createElement("div");
    actions.className = "actions";
    const add = (action, label) => {
      const b = document.createElement("button");
      b.textContent = t(label);
      b.disabled = !pageOk;
      b.addEventListener("click", () => act(action, d.node, b));
      actions.appendChild(b);
    };
    add("send_page", "popupSend");
    add("open_on", "popupOpen");
    if (d.can_download) add("download_on", "popupDownload");
    li.appendChild(actions);
    list.appendChild(li);
  }
}

async function load(fresh) {
  if (fresh) {
    say(t("working"));
    await api.runtime.sendMessage({ kind: "refresh" });
  }
  const { devices = [], devicesError = null } = await api.storage.local.get(["devices", "devicesError"]);
  render(devices, devicesError);
}

async function permissionButton() {
  const b = $("allow-images");
  const has = await api.permissions.contains({ origins: ["<all_urls>"] }).catch(() => true);
  b.hidden = has;
  b.onclick = async () => {
    const ok = await api.permissions.request({ origins: ["<all_urls>"] }).catch(() => false);
    b.hidden = ok;
  };
}

localize();
[tab] = await api.tabs.query({ active: true, currentWindow: true });
$("page-title").textContent = (tab && (tab.title || tab.url)) || "";
$("read-later").addEventListener("click", (e) => act("read_later", "", e.currentTarget));
$("refresh").addEventListener("click", () => load(true));
permissionButton();
await load(false);
load(true);
