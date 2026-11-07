// Screenshot mode: a stand-in for the Tauri backend, so the UI can run in a
// plain browser with SAMPLE data (made-up devices, notes, files...). Never
// the user's real data, never in the app: only src/dev/mockEntry.ts loads
// this, and only the `mock` Vite mode builds that entry (vite.config.ts).
//
// It installs `window.__TAURI_INTERNALS__` (invoke, callbacks, the event
// plugin) and answers each command from FIXTURES below, else from
// mockDefaults.json (an empty-but-well-formed answer for every command,
// generated from bindings.ts by scripts/gen-mock-defaults.mjs). Events the
// screens listen for (links coming up, device statuses, dialogs) are sent
// shortly after a screen starts listening.
//
// URL parameters (see scripts/screenshots-desktop.sh):
//   mock=1            required
//   view=<name>       home, device, notes, timers, readLater, linksMap,
//                     health, setup, activity, transfers, settings, help,
//                     search, developer, signIn (handled in App.tsx)
//   device=<id>       macbook, pixel8, officepc, lablaptop
//   window=<label>    quick-panel, kvm-overlay (other windows of the app)
//   step=<step>       setup's step (name, devices, features, permissions, connection)
//   dialog=<name>     keyChanged, permission, download, print, hotspot,
//                     call, ring, addDevice, sendTo
//   section=<id>      scroll Settings' #settings-<id> into view
//   scroll=<px>       scroll the page down this far
//   width=<px>        lay the page out this narrow (under Chrome's 500 px)
//   theme=light|dark, lang=en|hi|…

import defaults from "./mockDefaults.json";

const DEF = defaults as unknown as Record<string, Record<string, unknown>>;

type Args = Record<string, unknown>;
type Handler = (args: Args) => unknown;

const NOW = Date.now();
const MIN = 60_000;
const HOUR = 60 * MIN;
const DAY = 24 * HOUR;
/** Unix seconds, for the answers that use them (most use ms). */
const secs = (ms: number) => Math.round(ms / 1000);

// --- the sample world --------------------------------------------------------

export const ME = "n-home-mini";
export const MACBOOK = "macbook";
export const PIXEL = "pixel8";
export const OFFICE = "officepc";
export const LAB = "lablaptop";

const peer = (node_id: string, name: string, ip: string, source: "registry" | "lan" = "registry") => ({
  node_id,
  name,
  ip,
  tcp_port: 47100,
  udp_port: 47101,
  ws_port: 47102,
  quic_port: 47103,
  source,
  ws_open: false,
  inbound_blocked: false,
  nat: "easy",
});

const PEERS = [
  peer(MACBOOK, "Pankaj’s MacBook Pro", "192.168.1.24", "lan"),
  peer(PIXEL, "Pixel 8", "100.72.14.9"),
  peer(OFFICE, "Office PC", "10.20.0.41"),
  peer(LAB, "Lab laptop", "192.168.1.37", "lan"),
];

const KINDS: Record<string, { os: string; form: string; model: string; maker: string; icon: string }> = {
  [MACBOOK]: { os: "macos", form: "laptop", model: "MacBook Pro", maker: "Apple", icon: "mac_laptop" },
  [PIXEL]: { os: "android", form: "phone", model: "Pixel 8", maker: "Google", icon: "android_phone" },
  [OFFICE]: { os: "windows", form: "desktop", model: "OptiPlex 7010", maker: "Dell", icon: "win_desktop" },
  [LAB]: { os: "linux", form: "laptop", model: "ThinkPad X1 Carbon", maker: "Lenovo", icon: "linux_laptop" },
};

/** Live links: the MacBook and the lab laptop direct, the phone through the relay. */
const LINKS: Record<string, "quic" | "relay"> = { [MACBOOK]: "quic", [LAB]: "quic", [PIXEL]: "relay" };
const QUALITY: Record<string, "excellent" | "good" | "weak"> = { [MACBOOK]: "excellent", [LAB]: "good", [PIXEL]: "good" };

const GB = 1024 ** 3;

// Album art for Now Playing: a 48x48 PNG drawn for these samples (a moon
// over hills).
const ART =
  "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";

const STATUS: Record<string, Record<string, unknown>> = {
  [PIXEL]: {
    platform: "android",
    battery: 76,
    charging: false,
    actions: ["flashlight", "volume", "media", "ringer", "launch", "search", "dial", "keep_awake", "radio", "wifi", "locate", "key", "screen", "gallery", "screen_settings", "lost"],
    torch: false,
    volume: 40,
    ringer: "normal",
    wifi: { on: true, ssid: "Home-5G" },
    wifi_networks: [
      { ssid: "Home-5G", signal: 92, secure: true, saved: true, current: true },
      { ssid: "Home-2.4", signal: 71, secure: true, saved: true, current: false },
      { ssid: "Cafe Guest", signal: 38, secure: false, saved: false, current: false },
    ],
    bluetooth: true,
    mobile_data: true,
    hotspot: false,
    location_on: true,
    airplane: false,
    radio_direct: ["wifi", "bluetooth"],
    location_sharing: true,
    location: { lat: 18.5204, lon: 73.8567, acc: 12, at: secs(NOW - 6 * MIN) },
    helper: true,
    screen_send: true,
    screen_control: false,
    audio_source: true,
    audio_sink: true,
    camera_view: 1,
    insert_capture: 1,
    remote_download: 1,
    media_ctl: 1,
    brightness: 62,
    auto_brightness: true,
    screen_timeout_s: 60,
    auto_rotate: false,
    write_settings: true,
    dnd: false,
    shared_folders: 2,
    storage: { free: 61 * GB, total: 128 * GB },
    ram: { free: 3.1 * GB, total: 8 * GB },
    sync: 1,
    now_playing: {
      title: "Clair de Lune",
      artist: "Claude Debussy",
      album: "Suite bergamasque",
      app: "YouTube Music",
      state: "playing",
      duration: 312_000,
      position: 104_000,
      can_next: true,
      can_prev: true,
      can_seek: true,
      volume: 6,
      volume_max: 15,
      art: ART,
    },
  },
  [MACBOOK]: {
    battery: 54,
    charging: false,
    actions: ["volume", "mute", "media", "lock", "search", "slides", "input", "screen", "locate", "keep_awake", "run"],
    volume: 30,
    muted: false,
    storage: { free: 212 * GB, total: 494 * GB },
    ram: { free: 6.4 * GB, total: 16 * GB },
    cpu: 23,
    uptime_s: 3 * 86400 + 4 * 3600,
    sync: 1,
    remote_download: 1,
    media_ctl: 1,
    shared_folders: 1,
    commands: [
      { id: "c1", name: "Back up photos" },
      { id: "c2", name: "Empty Downloads" },
    ],
    last_run: { id: "c1", name: "Back up photos", code: 0, output: "Copied 214 files (1.8 GB)", timed_out: false, at: Math.round((NOW - 2 * HOUR) / 1000) },
    now_playing: {
      title: "Weekly design sync",
      artist: "Podcasts",
      app: "Podcasts",
      state: "paused",
      duration: 2_400_000,
      position: 1_250_000,
      can_next: true,
      can_prev: true,
      can_seek: true,
    },
  },
  [LAB]: {
    battery: 91,
    charging: false,
    actions: ["volume", "mute", "media", "lock", "search", "input", "screen", "keep_awake"],
    volume: 55,
    muted: false,
    storage: { free: 88 * GB, total: 256 * GB },
    ram: { free: 9.2 * GB, total: 32 * GB },
    cpu: 61,
    uptime_s: 11 * 3600,
    sync: 1,
    media_ctl: 1,
  },
};

// --- samples for screens -----------------------------------------------------

const BATTERY = (node: string) => {
  const base = node === PIXEL ? 76 : node === MACBOOK ? 54 : 91;
  return Array.from({ length: 48 }, (_, i) => {
    const at = secs(NOW - (47 - i) * 30 * MIN);
    // Down through the day, a charge overnight.
    const charging = i >= 10 && i < 18;
    // Down to `base` by now.
    const level = charging ? Math.min(100, 35 + (i - 10) * 8) : i < 10 ? Math.max(20, 60 - i * 3) : 100 - ((i - 18) * (100 - base)) / 29;
    return { at, level: Math.round(level), charging };
  });
};

const NOTES = [
  {
    id: "note-1",
    title: "Trip to Goa",
    body: "- [x] Book train tickets\n- [x] Hotel near Baga beach\n- [ ] Sunscreen\n- [ ] Charger for the camera\n- [ ] Print the itinerary",
    pinned: true,
    checklist: true,
    color: "teal",
    created_at: NOW - 6 * DAY,
    updated_at: NOW - 2 * HOUR,
    by: PIXEL,
    by_name: "Pixel 8",
    conflict_of: null,
    remind_at: NOW + DAY,
    remind_done: false,
  },
  {
    id: "note-2",
    title: "Wi-Fi for guests",
    body: "Network: Home-Guest\nPassword is on the fridge.\nThe printer is “Office LaserJet”.",
    pinned: true,
    checklist: false,
    color: "amber",
    created_at: NOW - 30 * DAY,
    updated_at: NOW - 3 * DAY,
    by: ME,
    by_name: "Home Mac mini",
    conflict_of: null,
    remind_at: null,
    remind_done: false,
  },
  {
    id: "note-3",
    title: "Groceries",
    body: "- [ ] Milk\n- [ ] Atta (5 kg)\n- [x] Tomatoes\n- [ ] Coffee beans\n- [ ] Dish soap",
    pinned: false,
    checklist: true,
    color: null,
    created_at: NOW - DAY,
    updated_at: NOW - 40 * MIN,
    by: PIXEL,
    by_name: "Pixel 8",
    conflict_of: null,
    remind_at: null,
    remind_done: false,
  },
  {
    id: "note-4",
    title: "Book ideas",
    body: "The Overstory — Richard Powers\nPiranesi — Susanna Clarke\nA Gentleman in Moscow",
    pinned: false,
    checklist: false,
    color: "violet",
    created_at: NOW - 12 * DAY,
    updated_at: NOW - 5 * DAY,
    by: MACBOOK,
    by_name: "Pankaj’s MacBook Pro",
    conflict_of: null,
    remind_at: null,
    remind_done: false,
  },
  {
    id: "note-5",
    title: "Meeting notes — Q4 plan",
    body: "Ship the new onboarding by the 20th.\nAsk Priya about the budget.\nNext review: Thursday 3 pm.",
    pinned: false,
    checklist: false,
    color: null,
    created_at: NOW - 2 * DAY,
    updated_at: NOW - 26 * HOUR,
    by: LAB,
    by_name: "Lab laptop",
    conflict_of: null,
    remind_at: NOW + 3 * HOUR,
    remind_done: false,
  },
];

const TIMERS = [
  {
    id: "timer-1",
    label: "Pasta",
    duration_ms: 10 * MIN,
    state: "running",
    ends_at: NOW + 6 * MIN + 12_000,
    remaining_ms: null,
    ring_on: [ME, PIXEL],
    created_at: NOW - 4 * MIN,
    updated_at: NOW - 4 * MIN,
    by: PIXEL,
    by_name: "Pixel 8",
  },
  {
    id: "timer-2",
    label: "Laundry",
    duration_ms: 45 * MIN,
    state: "paused",
    ends_at: 0,
    remaining_ms: 18 * MIN,
    ring_on: [PIXEL],
    created_at: NOW - 30 * MIN,
    updated_at: NOW - 3 * MIN,
    by: ME,
    by_name: "Home Mac mini",
  },
  {
    id: "timer-3",
    label: "Focus block",
    duration_ms: 50 * MIN,
    state: "running",
    ends_at: NOW + 37 * MIN,
    remaining_ms: null,
    ring_on: [ME, MACBOOK],
    created_at: NOW - 13 * MIN,
    updated_at: NOW - 13 * MIN,
    by: MACBOOK,
    by_name: "Pankaj’s MacBook Pro",
  },
];

const READ_LATER = [
  ["rl-1", "https://example.com/blog/rust-async-in-depth", "Async Rust in depth: futures, wakers and executors", "example.com", false, 2 * HOUR, "Pixel 8"],
  ["rl-2", "https://news.example.org/science/monsoon-forecast", "What this year’s monsoon forecast means for farmers", "news.example.org", false, 5 * HOUR, "Pixel 8"],
  ["rl-3", "https://design.example.net/articles/dark-mode-contrast", "Designing dark themes with enough contrast", "design.example.net", false, DAY, "Pankaj’s MacBook Pro"],
  ["rl-4", "https://recipes.example.com/masala-dosa", "Crispy masala dosa at home", "recipes.example.com", true, 2 * DAY, "Pixel 8"],
  ["rl-5", "https://example.com/talks/local-first-software", "Local-first software: you own your data", "example.com", true, 4 * DAY, "Lab laptop"],
].map(([id, url, title, host, read, ago, on]) => ({
  id: id as string,
  url: url as string,
  title: title as string,
  host: host as string,
  read: read as boolean,
  saved_at: NOW - (ago as number),
  saved_on: on as string,
  updated_at: NOW - (ago as number),
}));

const transfer = (
  id: string,
  node: string,
  name: string,
  incoming: boolean,
  state: "active" | "done" | "failed",
  total: number,
  done: number,
  ago: number,
  extra: Record<string, unknown> = {},
) => ({
  id,
  node_id: node,
  name,
  incoming,
  state,
  done,
  total,
  resumed_from: null,
  path: incoming && state === "done" ? `~/Downloads/${name}` : null,
  reason: null,
  started: NOW - ago - 30_000,
  ended: state === "active" ? null : NOW - ago,
  exists: true,
  via: null,
  link: null,
  host: null,
  stage: null,
  batch: null,
  kind: null,
  count: 0,
  files_done: 0,
  files_failed: 0,
  paused: false,
  rate: state === "active" ? 38 * 1024 * 1024 : null,
  eta: state === "active" ? 9 : null,
  ...extra,
});

const MB = 1024 * 1024;
const TRANSFERS = [
  transfer("t1", MACBOOK, "Holiday video.mov", false, "active", 1200 * MB, 860 * MB, 0),
  transfer("t2", PIXEL, "IMG_20261104_181522.jpg", true, "done", 4.2 * MB, 4.2 * MB, 12 * MIN),
  transfer("t3", PIXEL, "Boarding pass.pdf", true, "done", 380 * 1024, 380 * 1024, 2 * HOUR),
  transfer("t4", LAB, "Project report", false, "done", 86 * MB, 86 * MB, 3 * HOUR, { kind: "folder", count: 24, files_done: 24, batch: "b1" }),
  transfer("t5", OFFICE, "Invoice-October.xlsx", false, "failed", 2.1 * MB, 0.6 * MB, 5 * HOUR, { reason: "offline" }),
  transfer("t6", PIXEL, "ubuntu-24.04-desktop-amd64.iso", true, "done", 5.8 * GB, 5.8 * GB, DAY, { link: "https://releases.example.com/ubuntu-24.04-desktop-amd64.iso", host: "releases.example.com" }),
];

const act = (id: string, kind: string, action: string, node: string | null, ago: number, subject = "", detail = "", extra: Record<string, unknown> = {}) => ({
  id,
  kind,
  action,
  node,
  ts: NOW - ago,
  subject,
  detail,
  bytes: null,
  via: null,
  target: "",
  path: null,
  ...extra,
});

const ACTIVITY = [
  act("a1", "files", "file_sending", MACBOOK, 1 * MIN, "Holiday video.mov", "", { bytes: 1200 * MB }),
  act("a2", "clipboard", "clip_in", PIXEL, 6 * MIN, "Meet at the north gate at 6:30"),
  act("a3", "messages", "msg_in", PIXEL, 9 * MIN, "Leaving the office now 🚗"),
  act("a4", "files", "file_received", PIXEL, 12 * MIN, "IMG_20261104_181522.jpg", "", { bytes: 4.2 * MB, target: "transfers" }),
  act("a5", "calls", "call", PIXEL, 40 * MIN, "Mom", "missed"),
  act("a6", "notifications", "notification", PIXEL, 52 * MIN, "Bank: Your card ending 4821 was used for ₹1,250"),
  act("a7", "other", "timer", PIXEL, 70 * MIN, "Tea", String(4 * MIN)),
  act("a8", "connections", "link_up", LAB, 2 * HOUR),
  act("a9", "files", "file_received", PIXEL, 2 * HOUR, "Boarding pass.pdf", "", { bytes: 380 * 1024 }),
  act("a10", "other", "note", PIXEL, 3 * HOUR, "Trip to Goa"),
  act("a11", "files", "file_send_failed", OFFICE, 5 * HOUR, "Invoice-October.xlsx"),
  act("a12", "connections", "link_down", OFFICE, 6 * HOUR),
  act("a13", "other", "read_later", PIXEL, 7 * HOUR, "Async Rust in depth"),
  act("a14", "clipboard", "clip_out", MACBOOK, 26 * HOUR, "https://example.com/talks/local-first-software"),
];

const NOTIF_GROUPS = [
  {
    id: "g1",
    from: PIXEL,
    app: "Messages",
    pkg: "com.google.android.apps.messaging",
    title: "Asha",
    icon: null,
    avatar: null,
    group_chat: false,
    private: false,
    ts: NOW - 4 * MIN,
    items: [
      {
        id: 101,
        from: PIXEL,
        app: "Messages",
        pkg: "com.google.android.apps.messaging",
        title: "Asha",
        body: "Are we still on for dinner tonight?",
        key: "k101",
        buttons: [{ title: "Mark as read", reply: false }],
        reply: { title: "Reply", reply: true },
        conv: "asha",
        conv_title: "Asha",
        group_chat: false,
        sender: "Asha",
        messages: [
          { sender: "Asha", text: "Hey! Back from the trip?", ts: NOW - 20 * MIN },
          { sender: "Asha", text: "Are we still on for dinner tonight?", ts: NOW - 4 * MIN },
        ],
        avatar: null,
        picture: null,
        icon: null,
        private: false,
        silent: false,
        ts: NOW - 4 * MIN,
        handled: false,
      },
    ],
  },
  {
    id: "g2",
    from: PIXEL,
    app: "Calendar",
    pkg: "com.google.android.calendar",
    title: "Dentist appointment",
    icon: null,
    avatar: null,
    group_chat: false,
    private: false,
    ts: NOW - 15 * MIN,
    items: [
      {
        id: 102,
        from: PIXEL,
        app: "Calendar",
        pkg: "com.google.android.calendar",
        title: "Dentist appointment",
        body: "Tomorrow at 10:30 · Smile Dental Clinic",
        key: "k102",
        buttons: [{ title: "Snooze", reply: false }],
        reply: null,
        conv: null,
        conv_title: null,
        group_chat: false,
        sender: null,
        messages: [],
        avatar: null,
        picture: null,
        icon: null,
        private: false,
        silent: false,
        ts: NOW - 15 * MIN,
        handled: false,
      },
    ],
  },
  {
    id: "g3",
    from: PIXEL,
    app: "Delivery",
    pkg: "com.example.delivery",
    title: "Your order is on the way",
    icon: null,
    avatar: null,
    group_chat: false,
    private: false,
    ts: NOW - 32 * MIN,
    items: [
      {
        id: 103,
        from: PIXEL,
        app: "Delivery",
        pkg: "com.example.delivery",
        title: "Your order is on the way",
        body: "Arriving in about 20 minutes.",
        key: "k103",
        buttons: [],
        reply: null,
        conv: null,
        conv_title: null,
        group_chat: false,
        sender: null,
        messages: [],
        avatar: null,
        picture: null,
        icon: null,
        private: false,
        silent: true,
        ts: NOW - 32 * MIN,
        handled: false,
      },
    ],
  },
];

const HISTORY = [
  ["Messages", "Asha", "Are we still on for dinner tonight?", 4 * MIN],
  ["Calendar", "Dentist appointment", "Tomorrow at 10:30 · Smile Dental Clinic", 15 * MIN],
  ["Delivery", "Your order is on the way", "Arriving in about 20 minutes.", 32 * MIN],
  ["Bank", "Card used", "Your card ending 4821 was used for ₹1,250 at Fresh Mart.", 52 * MIN],
  ["Gmail", "Priya Sharma", "Q4 plan: draft attached, comments by Thursday please.", 3 * HOUR],
  ["Weather", "Rain expected", "Light rain from 4 pm. Take an umbrella.", 26 * HOUR],
].map(([app, title, body, ago], i) => ({
  id: 200 + i,
  from: PIXEL,
  app: app as string,
  title: title as string,
  body: body as string,
  key: null,
  ts: NOW - (ago as number),
  dismissed: i > 2,
}));

const CHAT = (node: string) => {
  const lines: [boolean, string, number][] =
    node === PIXEL
      ? [
          [true, "Can you send me the boarding pass?", 3 * HOUR],
          [false, "Sent it to your phone just now", 3 * HOUR - 2 * MIN],
          [true, "Got it, thanks!", 3 * HOUR - 3 * MIN],
          [true, "Leaving the office now 🚗", 9 * MIN],
        ]
      : node === MACBOOK
          ? [
              [false, "Starting the video export, it’s a big one", 40 * MIN],
              [true, "OK — I’ll pick it up from Transfers", 38 * MIN],
            ]
          : node === LAB
            ? [[true, "Build finished: all 412 tests passed ✅", 2 * HOUR]]
            : [];
  return {
    entries: lines.map(([incoming, text, ago], i) => ({
      id: `${node}-m${i}`,
      from_node: incoming ? node : null,
      to_node: incoming ? null : node,
      text,
      truncated: false,
      at: NOW - ago,
      status: incoming ? "received" : "delivered",
    })),
    more: false,
    kept: lines.length,
  };
};

const CLIPS = [
  ["Meet at the north gate at 6:30", PIXEL, "in", 6 * MIN],
  ["https://example.com/talks/local-first-software", MACBOOK, "out", 26 * HOUR],
  ["Order #48213-AX", PIXEL, "in", 2 * HOUR],
  ["pankaj@example.com", LAB, "out", 3 * HOUR],
  ["221B Baker Street, London", PIXEL, "in", DAY],
].map(([text, node, dir, ago], i) => ({
  id: 300 + i,
  text: text as string,
  truncated: false,
  from_node: dir === "in" ? (node as string) : null,
  to_node: dir === "out" ? (node as string) : null,
  direction: dir as "in" | "out",
  at: NOW - (ago as number),
}));

const check = (id: string, level: string, reason: string, extra: Record<string, unknown> = {}) => ({
  id,
  level,
  reason,
  node_id: null,
  name: null,
  detail: null,
  features: [],
  reconnect_ms: null,
  fix: null,
  ...extra,
});

const HEALTH = () => {
  const checks = [
    check("server", "ok", "ok"),
    check("push", "warn", "unconfirmed", { fix: "push_test" }),
    check("disk", "ok", "ok", { detail: String(212 * 1024) }),
    check("permission", "ok", "granted", { detail: "notifications", features: ["notifications"] }),
    check("permission", "bad", "denied", { detail: "screen_recording", features: ["media"], fix: "open_permission" }),
    check("permission", "ok", "granted", { detail: "accessibility", features: ["media"] }),
    check("permission", "warn", "not_determined", { detail: "location", features: ["find"], fix: "allow_permission" }),
    check("link", "ok", "direct", { node_id: MACBOOK, detail: "lan", reconnect_ms: 180 }),
    check("link", "warn", "relay", { node_id: PIXEL, detail: "internet:symmetric_nat", fix: "links_map", reconnect_ms: 1400 }),
    check("link", "info", "asleep", { node_id: OFFICE }),
    check("link", "ok", "direct", { node_id: LAB, detail: "lan", reconnect_ms: 240 }),
    check("version", "ok", "same", { node_id: MACBOOK }),
    check("version", "warn", "older", { node_id: OFFICE, detail: "0.9.2", features: ["notes", "timers"], fix: "update_other" }),
    check("allowed", "info", "allowed", { node_id: PIXEL, features: ["control", "screen"], fix: "permissions" }),
  ];
  return {
    checks,
    worst: "bad",
    bad: checks.filter((c) => c.level === "bad").length,
    warn: checks.filter((c) => c.level === "warn").length,
  };
};

const MAP = () => {
  const W = 760;
  const H = 460;
  const me = { node_id: ME, x: 380, y: 230, connected: true };
  const at: Record<string, [number, number]> = {
    [MACBOOK]: [150, 110],
    [PIXEL]: [620, 110],
    [OFFICE]: [620, 360],
    [LAB]: [150, 360],
  };
  const spots = Object.entries(at).map(([node_id, [x, y]]) => ({ node_id, x, y, connected: !!LINKS[node_id] }));
  const edge = (b: string, how: string, tone: string, label_key: string, latency: string | null, dash: string | null = null, why: unknown[] = []) => {
    const [x, y] = at[b];
    return {
      id: `${ME}-${b}`,
      a: ME,
      b,
      mine: true,
      how,
      tone,
      dash,
      path: `M ${me.x} ${me.y} L ${x} ${y}`,
      // Nearer the device than the middle, so the labels don't overlap.
      label_x: me.x + (x - me.x) * 0.6,
      label_y: me.y + (y - me.y) * 0.6,
      label_key,
      latency,
      nearby: false,
      why,
    };
  };
  const edges = [
    edge(MACBOOK, "lan", "lan", "map.how.lan", "4 ms"),
    edge(PIXEL, "relay", "relay", "map.how.relay", "48 ms", "6 4", [
      { path: "internet", reason: "symmetric_nat", path_key: "map.p.internet", what_key: "map.why.internet.symmetric_nat", fix_key: "map.fix.internet.symmetric_nat", ago_s: 120 },
    ]),
    edge(OFFICE, "none", "none", "map.how.none", null, "2 6"),
    edge(LAB, "lan", "lan", "map.how.lan", "7 ms"),
    {
      id: `${MACBOOK}-${LAB}`,
      a: MACBOOK,
      b: LAB,
      mine: false,
      how: "lan",
      tone: "lan",
      dash: null,
      path: `M 150 110 L 150 360`,
      label_x: 150,
      label_y: 235,
      label_key: "map.how.lan",
      latency: "3 ms",
      nearby: false,
      why: [],
    },
  ];
  const link = (how: string, transport: string, rtt: number) => ({
    how,
    transport,
    remote: null,
    addr_kind: how === "lan" ? "lan" : "public",
    rtt_ms: rtt,
    loss_pct: 0,
    since_s: 3600,
    heard_ago_s: 2,
    nearby_media: false,
    media_latency_ms: null,
  });
  const device = (node_id: string, l: unknown, state: string, verified: boolean) => ({
    node_id,
    name: PEERS.find((p) => p.node_id === node_id)!.name,
    source: "registry",
    link: l,
    state,
    reason: null,
    last_seen_ago_s: l ? 1 : 6 * 3600,
    shares_links: true,
    tries: [],
    problem: null,
    verified,
  });
  return {
    width: W,
    height: H,
    me,
    spots,
    edges,
    map: {
      node_id: ME,
      name: "Home Mac mini",
      devices: [
        device(MACBOOK, link("lan", "quic", 4), "connected", true),
        device(PIXEL, link("relay", "relay", 48), "connected", true),
        device(OFFICE, null, "asleep", false),
        device(LAB, link("lan", "quic", 7), "connected", false),
      ],
      others: [{ a: MACBOOK, b: LAB, how: "lan", rtt_ms: 3, reported_by: MACBOOK, reported_ago_s: 30 }],
    },
  };
};

const SEARCH = (query: string) => {
  const q = query.toLowerCase();
  const hit = (id: string, title: string, detail: string, node: string | null, ago: number, tag: string | null = null) => ({
    id,
    title,
    detail,
    node,
    ts: NOW - ago,
    tag,
    score: 1,
  });
  const all: [string, ReturnType<typeof hit>][] = [
    ["messages", hit("pixel8-m0", "Can you send me the boarding pass?", "Pixel 8", PIXEL, 3 * HOUR)],
    ["notes", hit("note-2", "Wi-Fi for guests", "Network: Home-Guest · Password is on the fridge.", ME, 3 * DAY)],
    ["notes", hit("note-1", "Trip to Goa", "Book train tickets · Hotel near Baga beach", PIXEL, 2 * HOUR)],
    ["transfers", hit("t3", "Boarding pass.pdf", "From Pixel 8 · 380 KB", PIXEL, 2 * HOUR)],
    ["clipboard", hit("304", "221B Baker Street, London", "From Pixel 8", PIXEL, DAY)],
    ["read_later", hit("rl-2", "What this year’s monsoon forecast means for farmers", "news.example.org", PIXEL, 5 * HOUR)],
    ["notifications", hit("203", "Card used", "Bank · Your card ending 4821 was used", PIXEL, 52 * MIN)],
    ["settings", hit("key", "Device key and passphrase", "What encrypts everything your devices send", null, 0)],
    ["help", hit("security", "Security and privacy", "Passphrase, device keys, what the server sees", null, 0)],
  ];
  const groups: { kind: string; total: number; hits: ReturnType<typeof hit>[] }[] = [];
  for (const [kind, h] of all) {
    if (!q || !`${h.title} ${h.detail}`.toLowerCase().includes(q)) continue;
    let g = groups.find((x) => x.kind === kind);
    if (!g) groups.push((g = { kind, total: 0, hits: [] }));
    g.hits.push(h);
    g.total++;
  }
  return { groups };
};

const PERMS = [
  { node_id: PIXEL, control: true, screen: true, kvm: false },
  { node_id: MACBOOK, control: true, screen: false, kvm: true },
  { node_id: LAB, control: false, screen: false, kvm: false },
  { node_id: OFFICE, control: false, screen: false, kvm: false },
];

const SCHEDULES = [
  { id: "s1", node: PIXEL, action: "ringer", args: '{"mode":"silent"}', label: "Silent at night", time: "22:30", days: [0, 1, 2, 3, 4, 5, 6], enabled: true, last_run: null },
  { id: "s2", node: PIXEL, action: "ringer", args: '{"mode":"normal"}', label: "Ring in the morning", time: "07:00", days: [1, 2, 3, 4, 5], enabled: true, last_run: null },
  { id: "s3", node: ME, action: "lock", args: "{}", label: "Lock at the end of the day", time: "19:00", days: [1, 2, 3, 4, 5], enabled: false, last_run: null },
];

const AUTOMATIONS = [
  { id: "au1", trigger: { kind: "battery_below", node: PIXEL, level: 20 }, node: ME, action: "notify", args: "{}", label: "Tell me when the phone is low", enabled: true, last_fired: null },
  { id: "au2", trigger: { kind: "connected", node: PIXEL, level: null }, node: ME, action: "mute", args: '{"on":false}', label: "Unmute when I’m home", enabled: true, last_fired: null },
];

const SHORTCUTS = {
  bindings: [
    { binding: { id: "k1", action: "send_clipboard", node: PIXEL, automation: "", keys: "Cmd+Shift+V", enabled: true }, label: "⌘⇧V", name: "Pixel 8", problem: null },
    { binding: { id: "k2", action: "ring", node: PIXEL, automation: "", keys: "Cmd+Shift+F", enabled: true }, label: "⌘⇧F", name: "Pixel 8", problem: null },
    { binding: { id: "k3", action: "insert_photo", node: "", automation: "", keys: "Cmd+Shift+P", enabled: false }, label: "⌘⇧P", name: "", problem: null },
  ],
  unavailable: false,
  mac: true,
  saved: "",
};

// --- command answers ---------------------------------------------------------

const ready = () => new URLSearchParams(location.search).get("view") !== "signIn";
const dialog = () => new URLSearchParams(location.search).get("dialog") ?? "";

const FIXTURES: Record<string, Handler> = {
  is_ready: () => ready(),
  get_saved_session: () => ({ server_url: "https://sync.example.com", username: "pankaj" }),
  session_info: () => ({ username: "pankaj", server_url: "https://sync.example.com", has_key: true }),
  account_sessions: () => [
    { id: "s-me", name: "Home Mac mini", node_id: ME, created: secs(NOW - 40 * DAY), last_seen: secs(NOW - MIN), current: true },
    { id: "s-mac", name: "Pankaj’s MacBook Pro", node_id: MACBOOK, created: secs(NOW - 38 * DAY), last_seen: secs(NOW - 5 * MIN), current: false },
    { id: "s-pixel", name: "Pixel 8", node_id: PIXEL, created: secs(NOW - 20 * DAY), last_seen: secs(NOW - 2 * HOUR), current: false },
  ],
  get_identity: () => ({ ...peer(ME, "Home Mac mini", "192.168.1.10") }),
  get_peers: () => PEERS,
  refresh_from_server: () => null,
  device_kinds: () => KINDS,
  own_device_kind: () => ({ os: "macos", form: "desktop", model: "Mac mini", maker: "Apple", icon: "mac_desktop" }),
  get_device_labels: () => ({ [PIXEL]: { color: "teal" } }),
  link_stats: () =>
    Object.entries(LINKS).map(([node_id, transport]) => ({
      node_id,
      transport,
      direct: transport !== "relay",
      rtt_ms: transport === "relay" ? 48 : 4,
      loss_pct: 0,
      quality: QUALITY[node_id],
      label: "",
    })),
  clipboard_sync_enabled: () => true,
  clip_history_enabled: () => true,
  chat_history_enabled: () => true,
  notifications_enabled: () => true,
  quick_panel_enabled: () => true,
  labels_share: () => true,
  share_location: () => true,
  location_available: () => true,
  otp_auto_copy: () => false,
  phone_quiet: () => [],
  request_status: (a) => {
    const node = a.nodeId as string;
    if (STATUS[node]) later(() => emit("device-status-event", { from: node, status: STATUS[node] }), 30);
    return "direct";
  },
  battery_history: (a) => (STATUS[a.node as string]?.battery !== undefined ? BATTERY(a.node as string) : []),
  get_page_layout: (a) => ({ order: a.tiles as string[], hidden: [], scope: "default" }),
  notes_list: () => NOTES,
  timers_list: () => TIMERS,
  timer_devices: () => [ME, PIXEL, MACBOOK, LAB],
  read_later_list: () => READ_LATER,
  transfer_list: () => TRANSFERS,
  activity_feed: () => ({ items: ACTIVITY, more: false, hidden: [], cleared_at: null }),
  phone_notifications: (a) => (a.nodeId === PIXEL ? NOTIF_GROUPS : []),
  phone_notif_prefs: () => ({ private_mode: false, only_locked: false }),
  notification_history: (a) =>
    a.nodeId === PIXEL ? { entries: HISTORY, total: HISTORY.length, kept: 500, apps: ["Bank", "Calendar", "Delivery", "Gmail", "Messages", "Weather"] } : { entries: [], total: 0, kept: 500, apps: [] },
  muted_apps: () => ["Weather"],
  chat_history: (a) => CHAT(a.nodeId as string),
  clip_history: () => ({ entries: CLIPS, total: CLIPS.length, kept: 200, pictures: [] }),
  clipboard_history: () => JSON.stringify(CLIPS.map((c) => ({ text: c.text, from: c.from_node ?? undefined, ts: c.at }))),
  quick_panel_clips: () => CLIPS,
  health_check: () => HEALTH(),
  links_map: () => MAP(),
  search: (a) => SEARCH((a.query as string) ?? ""),
  onboarding_state: () => {
    const step = new URLSearchParams(location.search).get("step") || "devices";
    return { step, finished: !new URLSearchParams(location.search).get("step"), skipped_all: false, skipped: [], features: ["clipboard", "notifications", "files"] };
  },
  key_changes: () =>
    dialog() === "keyChanged"
      ? [
          {
            node_id: PIXEL,
            name: "Pixel 8",
            old_fingerprint: "4F2A 91C3 07BE 55D1",
            new_fingerprint: "B7E0 3C18 A9F2 6D44",
            first_ago_s: 300,
            last_ago_s: 20,
            attempts: 3,
            dismissed: false,
            was_verified: true,
          },
        ]
      : [],
  key_verification: (a) => {
    const node = a.nodeId as string;
    const p = PEERS.find((x) => x.node_id === node);
    if (!p) return null;
    return {
      node_id: node,
      name: p.name,
      own_fingerprint: "8C41 2E9B F013 77AD",
      their_fingerprint: "4F2A 91C3 07BE 55D1",
      safety_number: "31270 88412 05593 61148",
      verified: node === MACBOOK || node === PIXEL,
      key_changed: false,
      code: "482913",
      qr_size: 0,
      qr_path: "",
    };
  },
  own_fingerprint: () => "8C41 2E9B F013 77AD",
  peer_permissions: (a) => PERMS.find((p) => p.node_id === a.nodeId) ?? { node_id: a.nodeId, control: false, screen: false, kvm: false },
  all_permissions: () => PERMS,
  list_schedules: () => SCHEDULES,
  list_automations: () => AUTOMATIONS,
  list_shortcuts: () => SHORTCUTS,
  local_actions: () => ({ node: ME, actions: ["lock", "mute", "media", "keep_awake", "volume"] }),
  list_shell_commands: () => [
    { id: "c1", name: "Back up photos", command: "rsync -a ~/Pictures /Volumes/Backup/", enabled: true },
    { id: "c2", name: "Empty Downloads", command: "rm -rf ~/Downloads/tmp/*", enabled: false },
  ],
  webhook_settings: () => ({ enabled: true, port: 47321, token: "sample-token-0000-1111", running: true, error: null }),
  sync_pairs: () => [
    { id: "sp1", peer: MACBOOK, name: "Documents", path: "~/Documents", mode: "both", paused: false, state: "idle", syncing: 0, files: 1284, conflicts: 0, skipped: 0, renamed: 0, last_synced: secs(NOW - 4 * MIN), error: null },
    { id: "sp2", peer: PIXEL, name: "Camera", path: "~/Pictures/Phone camera", mode: "receive", paused: false, state: "syncing", syncing: 3, files: 5210, conflicts: 1, skipped: 0, renamed: 0, last_synced: secs(NOW - 30 * MIN), error: null },
  ],
  sync_max_size_mb: () => 2048,
  removed_devices: () => [
    { node_id: "old-tab", name: "Old Galaxy Tab", fingerprint: "11AA 22BB 33CC 44DD", removed_ago_s: 9 * 86400, by_me: true, by_name: "Home Mac mini" },
  ],
  update_status: () => ({
    current: "1.4.0",
    disabled: null,
    auto_install: true,
    channel: "stable",
    phase: "idle",
    last_check: secs(NOW - 3 * HOUR),
    error: null,
    available: null,
    later: false,
    downloaded: null,
    total: null,
  }),
  calls_status: () => ({
    ...DEF.calls_status,
    available: true,
    enabled: true,
    device: "AA:BB:CC:DD:EE:01",
    device_name: "Pixel 8",
    connected: true,
    phase: dialog() === "call" ? "incoming" : "idle",
    number: dialog() === "call" ? "+91 98765 43210" : null,
    name: dialog() === "call" ? "Asha" : null,
    signal: 4,
    battery: 4,
    os: "macos",
  }),
  calls_devices: () => [{ address: "AA:BB:CC:DD:EE:01", name: "Pixel 8", connected: true }],
  computer_audio_status: () => ({ supported: true, playing_to: [], asked: [], via_bluetooth: [], error: null }),
  phone_audio_status: () => ({ ...DEF.phone_audio_status, state: "idle", volume: 80 }),
  media_supported: () => true,
  media_watch: () => true,
  insert_supported: () => true,
  can_wake: (a) => a.nodeId === OFFICE,
  wake_setting: () => ({ on: true, ifaces: [{ mac: "a4:83:e7:12:34:56", broadcast: "192.168.1.255", iface: "en0", wired: true }] }),
  nearby_media: () => true,
  nearby_status: (a) => ({ node_id: a.nodeId, media: true, near: a.nodeId === MACBOOK ? "lan" : null, low_latency: a.nodeId === MACBOOK, path: "lan", latency_ms: 40, before_ms: 180, rtt_ms: 4 }),
  verification_codes: () => [{ id: "otp1", from: PIXEL, code: "482 913", app: "Messages", issuer: "Example Bank", expires_in_ms: 4 * MIN }],
  kvm_settings: () => ({
    supported: true,
    blocked: null,
    enabled: true,
    clip_follows: true,
    swap_mods: false,
    hotkey: "Ctrl+Alt+K",
    layout: [{ node_id: MACBOOK, side: "left" }],
    computers: [
      { node_id: MACBOOK, name: "Pankaj’s MacBook Pro", os: "macos", linked: true, allowed: true },
      { node_id: LAB, name: "Lab laptop", os: "linux", linked: true, allowed: false },
      { node_id: OFFICE, name: "Office PC", os: "windows", linked: false, allowed: false },
    ],
    state: { role: "idle", node_id: null, name: null, edge: null, pos: null, reason: null },
    os: "macos",
  }),
  tray_battery_setting: () => ({ on: true, badge: true, phone: PIXEL, phones: [{ node: PIXEL, name: "Pixel 8" }] }),
  battery_alerts: () => ({ enabled: true, low: 20, charged: 90 }),
  away_lock: () => ({ on: true, minutes: 5, can_lock: true }),
  follow_phone_dnd: () => true,
  shared_folders: () => [{ id: "f1", name: "Shared", path: "~/Shared" }],
  remote_download_targets: () => [PIXEL, MACBOOK, LAB],
  remote_downloads: () =>
    dialog() === "download"
      ? [
          {
            id: "rd1",
            node_id: PIXEL,
            here: true,
            url: "https://releases.example.com/sample-installer-2.3.dmg",
            host: "releases.example.com",
            name: "sample-installer-2.3.dmg",
            size: 184 * MB,
            done: 0,
            state: "asking",
            error: null,
            path: null,
            started_at: NOW - 5000,
          },
        ]
      : [],
  permission_notice: () => null,
  key_rotation: () => ({ state: "none",by_me: false, by_name: "", devices: [], expires_in_s: 0, needs_passphrase: false, mismatch_name: "" }),
  passphrase_strength: () => 3,
  join_invite: () => ({ link: "mydomain://join?c=SAMPLE", code: "418 207", expires_in: 300, qr_size: 0, qr_path: "" }),
  join_status: () => ({ state: "waiting", name: null }),
  generate_key: () => "maple-river-cobalt-lantern-seven",
  dev_credentials: () => null,
  mute_here: () => ({ supported: true, on: false, active: false, why: null }),
  audio_route: () => "phone",
  audio_via: () => "link",
  diagnose: (a) => ({
    ...DEF.diagnose,
    node_id: a.nodeId,
    name: PEERS.find((p) => p.node_id === a.nodeId)?.name ?? "",
    verdict: "ok",
  }),
  list_gallery: () => "req-gallery",
  list_folder: () => "req-folder",
  take_send_request: () => (dialog() === "sendTo" ? { paths: ["/Users/sample/Desktop/Holiday photos.zip"], names: ["Holiday photos.zip"], size: 82 * MB } : null),
};

// --- events ------------------------------------------------------------------

type Cb = (data: unknown) => void;
const callbacks = new Map<number, Cb>();
const listeners = new Map<string, number[]>();
let nextId = 1;

function later(f: () => void, ms = 0) {
  setTimeout(f, ms);
}

/** Send an event to every listener of `event`. */
export function emit(event: string, payload: unknown) {
  for (const id of listeners.get(event) ?? []) callbacks.get(id)?.({ event, id, payload });
}

/** Events sent once, when a screen first listens for them. */
const ON_LISTEN: Record<string, () => void> = {
  "peer-connected": () => {
    for (const [node_id, transport] of Object.entries(LINKS)) emit("peer-connected", { node_id, transport });
    emit("connect-progress", { node_id: OFFICE, stage: "asleep" });
    // What each connected device last said about itself (battery...).
    later(() => {
      for (const node of Object.keys(LINKS)) if (STATUS[node]) emit("device-status-event", { from: node, status: STATUS[node] });
    }, 50);
  },
  "permission-ask": () => dialog() === "permission" && emit("permission-ask", { from: PIXEL, perm: "control" }),
  "hotspot-ask": () => dialog() === "hotspot" && emit("hotspot-ask", { sid: "h1", node_id: PIXEL, wifi: "Home-5G" }),
  "print-ask": () =>
    dialog() === "print" && emit("print-ask", { id: "p1", node_id: PIXEL, name: "Boarding pass.pdf", printer: "Office LaserJet", copies: 1 }),
  "ring-event": () => dialog() === "ring" && emit("ring-event", { from: PIXEL }),
  // Home's "Recent": a few files and messages from this session.
  "file-sent": () => {
    if (new URLSearchParams(location.search).get("view") !== "home") return;
    later(() => {
      emit("file-received", { id: "r1", from: PIXEL, name: "IMG_2041.jpg", path: "/Users/you/Downloads/IMG_2041.jpg", size: 2_400_000 });
      emit("message-received", { ok: true, node_id: PIXEL, text: "Leaving now, see you at 7", protocol: "quic" });
      emit("file-sent", { id: "r2", node_id: MACBOOK, name: "Quarterly report.pdf" });
      emit("message-received", { ok: true, node_id: LAB, text: "Build finished: all tests passed", protocol: "quic" });
    }, 80);
  },
};
const fired = new Set<string>();

function listen(args: Args): number {
  const event = args.event as string;
  const id = args.handler as number;
  listeners.set(event, [...(listeners.get(event) ?? []), id]);
  if (ON_LISTEN[event] && !fired.has(event)) {
    fired.add(event);
    later(ON_LISTEN[event], 50);
  }
  return id;
}

// --- install -----------------------------------------------------------------

async function invoke(cmd: string, args: Args = {}): Promise<unknown> {
  if (cmd === "plugin:event|listen") return listen(args);
  if (cmd === "plugin:event|unlisten" || cmd === "plugin:event|emit" || cmd === "plugin:event|emit_to") return null;
  if (cmd.startsWith("plugin:")) return null;
  const f = FIXTURES[cmd];
  if (f) return f(args);
  const d = DEF[cmd];
  // A fresh copy: screens may change what they get.
  return d === undefined ? null : structuredClone(d);
}

export function installMock(params: URLSearchParams) {
  const label = params.get("window") ?? "main";
  const w = window as unknown as Record<string, unknown>;
  w.__TAURI_INTERNALS__ = {
    invoke,
    transformCallback: (cb: Cb, once = false) => {
      const id = nextId++;
      callbacks.set(id, (data) => {
        if (once) callbacks.delete(id);
        cb(data);
      });
      return id;
    },
    unregisterCallback: (id: number) => callbacks.delete(id),
    runCallback: (id: number, data: unknown) => callbacks.get(id)?.(data),
    callbacks,
    convertFileSrc: (path: string) => path,
    metadata: { currentWindow: { label }, currentWebview: { windowLabel: label, label } },
    plugins: { path: { sep: "/", delimiter: ":" } },
  };
  w.__TAURI_EVENT_PLUGIN_INTERNALS__ = {
    unregisterListener: (event: string, id: number) => {
      callbacks.delete(id);
      listeners.set(
        event,
        (listeners.get(event) ?? []).filter((x) => x !== id),
      );
    },
  };
  w.__MD_MOCK__ = true;

  // Still pictures: no entrance animation caught half-way.
  const still = document.createElement("style");
  still.textContent = "* { animation-duration: 0s !important; animation-delay: 0s !important; transition: none !important; }";
  document.head.appendChild(still);
  // The quick panel's own window is 360 px wide (headless Chrome's window
  // can't be that narrow).
  // ?width=<px>: the page laid out that narrow (headless Chrome's window
  // can't go under 500 px); the screenshot is cropped to it.
  const narrow = Number(params.get("width") ?? 0);
  if (narrow) {
    document.documentElement.style.width = `${narrow}px`;
    document.documentElement.style.overflowX = "hidden";
  }
  if (label === "quick-panel") {
    document.documentElement.style.width = "360px";
    document.documentElement.style.height = "540px";
  }

  // Theme and language before the app reads them.
  const theme = params.get("theme");
  try {
    if (theme) localStorage.setItem("md.theme", theme === "dark" ? "cosmic-blue" : theme);
    const lang = params.get("lang");
    if (lang) localStorage.setItem("md.language", lang);
    else localStorage.removeItem("md.language");
  } catch {
    /* no storage: defaults */
  }

  // How tall the page is once drawn (the screenshot script reads it with
  // --dump-dom to know how far down to scroll).
  setTimeout(() => {
    const main = document.getElementById("main");
    document.body.dataset.mdHeight = String(main?.scrollHeight ?? document.documentElement.scrollHeight);
  }, 2500);

  // Once drawn: scroll where asked, so a screenshot shows that part.
  const section = params.get("section");
  const scroll = Number(params.get("scroll") ?? 0);
  if (section || scroll) {
    setTimeout(() => {
      const main = document.getElementById("main") ?? document.scrollingElement;
      if (section) {
        const el = document.getElementById(section.startsWith("settings-") ? section : `settings-${section}`);
        el?.scrollIntoView({ block: "start" });
      } else if (main) {
        main.scrollTop = scroll;
        window.scrollTo(0, scroll);
      }
    }, 1500);
  }
}
