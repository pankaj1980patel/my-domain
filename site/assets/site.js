// my-domain site: screenshot gallery, platform tabs, Android media from a manifest,
// lightbox, active nav link and gentle reveal. No dependencies.
(() => {
  "use strict";
  document.documentElement.classList.add("js");
  const $ = (s, r = document) => r.querySelector(s);
  const $$ = (s, r = document) => Array.from(r.querySelectorAll(s));
  const reduced = matchMedia("(prefers-reduced-motion: reduce)").matches;

  // ---------- Desktop gallery ----------
  const SHOTS = [
    ["overview", "Overview", "Your devices at a glance, quick actions and recent activity."],
    ["devices", "All devices", "Every phone and computer, its status, battery and how it’s connected."],
    ["device", "A phone’s page", "Battery, storage, health, permissions and quick actions for one device."],
    ["messages", "Messages", "Encrypted chat between your own devices."],
    ["remote", "Remote control", "Find and ring the phone, dial from the computer, set its volume and brightness."],
    ["media", "Media", "What’s playing on the phone, with controls and volume."],
    ["connections", "Connections", "A live map of how your devices reach each other."],
    ["transfers", "Files and transfers", "Drop files to send; transfers pause, resume and survive restarts."],
    ["notes", "Notes", "A shared notepad with reminders, the same on every device."],
    ["timers", "Timers", "Shared timers that ring on the devices you pick."],
    ["automations", "Automations", "When something happens on one device, do something on another."],
    ["health", "Security and health", "A health check with one fix for each problem."],
    ["appearance", "Appearance", "Five themes, from Cosmic Blue to Cosmic Light."],
    ["search", "Search", "Search messages, notes, files, settings and help in one place."],
  ];
  const img = $("#gal-img");
  const cap = $("#gal-cap");
  const thumbs = $("#gal-thumbs");
  let idx = 0;
  let theme = "dark";
  let want = "";
  const src = (slug, t, thumb) => `media/desktop/${slug}-${t}${thumb ? "-thumb" : ""}.webp`;
  const alt = (i, t) => `${SHOTS[i][1]} screen of the my-domain desktop app, ${t} theme, with sample data: ${SHOTS[i][2]}`;

  if (img && thumbs) {
    SHOTS.forEach(([slug, title], i) => {
      const li = document.createElement("li");
      const b = document.createElement("button");
      b.type = "button";
      b.dataset.i = String(i);
      b.innerHTML = `<img src="${src(slug, theme, true)}" width="400" height="256" alt="" loading="lazy"><span>${title}</span>`;
      b.addEventListener("click", () => show(i, true));
      li.appendChild(b);
      thumbs.appendChild(li);
    });

    const show = (i, user) => {
      idx = (i + SHOTS.length) % SHOTS.length;
      const [slug, title, text] = SHOTS[idx];
      const next = src(slug, theme);
      want = next;
      if (img.getAttribute("src") !== next) {
        img.classList.add("is-loading");
        const pre = new Image();
        // Only the latest choice wins, however the downloads finish.
        pre.onload = pre.onerror = () => { if (want !== next) return; img.src = next; img.classList.remove("is-loading"); };
        pre.src = next;
      } else {
        img.classList.remove("is-loading");
      }
      img.alt = alt(idx, theme);
      cap.innerHTML = `<strong>${title}</strong>${text}`;
      $$("button", thumbs).forEach((b) => {
        const on = Number(b.dataset.i) === idx;
        if (on) b.setAttribute("aria-current", "true"); else b.removeAttribute("aria-current");
        if (on && user) {
          // Keep the chosen thumbnail in view without moving the page.
          const list = thumbs, li = b.parentElement;
          if (list.scrollWidth > list.clientWidth) list.scrollTo({ left: li.offsetLeft - list.clientWidth / 2 + li.clientWidth / 2, behavior: reduced ? "auto" : "smooth" });
          else list.scrollTo({ top: li.offsetTop - list.clientHeight / 2 + li.clientHeight / 2, behavior: reduced ? "auto" : "smooth" });
        }
      });
    };

    $("#gal-prev").addEventListener("click", () => show(idx - 1, true));
    $("#gal-next").addEventListener("click", () => show(idx + 1, true));
    $(".gal").addEventListener("keydown", (e) => {
      if (e.target.closest(".seg")) return;
      if (e.key === "ArrowRight") { show(idx + 1, true); e.preventDefault(); }
      if (e.key === "ArrowLeft") { show(idx - 1, true); e.preventDefault(); }
    });
    $$("[data-theme-btn]").forEach((b) => b.addEventListener("click", () => {
      theme = b.dataset.themeBtn;
      $$("[data-theme-btn]").forEach((x) => x.setAttribute("aria-pressed", String(x === b)));
      $$("button", thumbs).forEach((t) => { t.querySelector("img").src = src(SHOTS[Number(t.dataset.i)][0], theme, true); });
      show(idx, false);
    }));
    $("#gal-zoom").addEventListener("click", () => openLightbox(src(SHOTS[idx][0], theme), alt(idx, theme)));
    show(0, false);
  }

  // ---------- Lightbox ----------
  const box = $("#lightbox");
  const boxImg = $("#lightbox-img");
  function openLightbox(href, text) {
    if (!box || typeof box.showModal !== "function") { window.open(href, "_blank", "noopener"); return; }
    boxImg.src = href;
    boxImg.alt = text;
    box.showModal();
  }
  if (box) box.addEventListener("click", (e) => { if (e.target === box) box.close(); });

  // ---------- Tabs (Desktop / Android) ----------
  const tabs = $$('[role="tab"]');
  const select = (tab, focus) => {
    tabs.forEach((t) => {
      const on = t === tab;
      t.setAttribute("aria-selected", String(on));
      t.tabIndex = on ? 0 : -1;
      $("#" + t.getAttribute("aria-controls")).hidden = !on;
    });
    if (focus) tab.focus();
  };
  tabs.forEach((t, i) => {
    t.addEventListener("click", () => select(t, false));
    t.addEventListener("keydown", (e) => {
      let j = null;
      if (e.key === "ArrowRight") j = (i + 1) % tabs.length;
      if (e.key === "ArrowLeft") j = (i - 1 + tabs.length) % tabs.length;
      if (e.key === "Home") j = 0;
      if (e.key === "End") j = tabs.length - 1;
      if (j !== null) { e.preventDefault(); select(tabs[j], true); }
    });
  });

  // ---------- Android media (site/media/android/manifest.json, written by tools/import-android.sh) ----------
  const esc = (s) => String(s).replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[c]));
  fetch("media/android/manifest.json", { cache: "no-cache" })
    .then((r) => (r.ok ? r.json() : null))
    .then((m) => {
      if (!m) return;
      const screens = Array.isArray(m.screens) ? m.screens : [];
      const videos = Array.isArray(m.videos) ? m.videos : [];
      const grid = $("#android-grid");
      if (screens.length && grid) {
        screens.forEach((s) => {
          const li = document.createElement("li");
          const file = "media/android/" + s.file;
          const text = s.alt || `${s.title} screen of the my-domain Android app, with sample data.`;
          li.innerHTML = `<figure><button type="button" class="phone" aria-label="Open ${esc(s.title)} larger"><img src="${esc(file)}" alt="${esc(text)}" loading="lazy" ${s.width ? `width="${s.width}" height="${s.height}"` : ""}></button><figcaption>${esc(s.title)}</figcaption></figure>`;
          li.querySelector("button").addEventListener("click", () => openLightbox(file, text));
          grid.appendChild(li);
        });
        $("#android-soon").hidden = true;
      }
      if (videos.length) {
        const wrap = $("#android-vids");
        videos.forEach((v) => {
          const f = document.createElement("figure");
          f.className = "vid vid--phone";
          f.innerHTML = `<video controls muted playsinline preload="none" ${v.poster ? `poster="media/android/${esc(v.poster)}"` : ""}><source src="media/android/${esc(v.file)}" type="video/mp4"></video><figcaption><strong>${esc(v.title || "Android walkthrough")}</strong>${esc(v.caption || "The Android app, with sample data.")}</figcaption>`;
          wrap.appendChild(f);
        });
        const note = $("#android-vid-note");
        if (note) note.hidden = true;
      }
    })
    .catch(() => { /* No manifest (or opened from disk): keep the "coming soon" state. */ });

  // ---------- Active section in the nav ----------
  const links = new Map($$(".nav a").map((a) => [a.getAttribute("href").slice(1), a]));
  if ("IntersectionObserver" in window) {
    const seen = new Map();
    const io = new IntersectionObserver((entries) => {
      entries.forEach((e) => seen.set(e.target.id, e.isIntersecting ? e.intersectionRatio : 0));
      let best = null, ratio = 0;
      seen.forEach((r, id) => { if (r > ratio) { ratio = r; best = id; } });
      links.forEach((a, id) => { if (id === best) a.setAttribute("aria-current", "true"); else a.removeAttribute("aria-current"); });
    }, { rootMargin: "-30% 0px -55% 0px", threshold: [0, 0.01, 0.25, 0.5, 1] });
    links.forEach((_, id) => { const s = document.getElementById(id); if (s) io.observe(s); });

    // ---------- Reveal on scroll ----------
    if (!reduced) {
      const els = $$(".sec__head, .feat, .chips, .gal, .vids, .diagram, .steps, .priv, .plat, .road");
      els.forEach((el) => el.classList.add("reveal"));
      const ro = new IntersectionObserver((entries) => entries.forEach((e) => {
        if (e.isIntersecting) { e.target.classList.add("is-in"); ro.unobserve(e.target); }
      }), { rootMargin: "0px 0px -8% 0px" });
      els.forEach((el) => ro.observe(el));
    }
  }
})();
