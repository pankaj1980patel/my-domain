// A device drawn as itself: a hand-made frame (phone, tablet, laptop or
// monitor, picked from its device kind) with its wallpaper on the screen,
// the way the device pages show your devices. What's on the screen, in
// order (mdcore `features::wallpaper`, backend `wallpaper.rs`):
//  1. the picture it shared (cover);
//  2. a gradient of the colours it shared;
//  3. a built-in wallpaper: original abstract art in the app's deep-space
//     colours (flowing ribbons, aurora glows), one of several, always the
//     same one for the same device.
// No line icon as the art. The frames and the built-in art are CSS and
// inline SVG; no images, no brand artwork.
//
// What each device shared comes from the backend (`wallpapers`, then
// `wallpaper-changed` events), kept here once for every tile.
//
// Also `WallpaperShareSetting`, the "Share my wallpaper with my devices"
// switch, for Settings.

import { createSignal, createUniqueId, For, onMount, Show, type JSX } from "solid-js";
import { createStore, reconcile } from "solid-js/store";
import { listen } from "@tauri-apps/api/event";
import { commands, type Wallpaper_Serialize } from "../bindings";
import { friendlyError } from "../lib/words";
import { toast } from "./ui";
import { t } from "../i18n";

/** The key this computer's own look is kept under. */
const OWN = "";

const [looks, setLooks] = createStore<Record<string, Wallpaper_Serialize | undefined>>({});
let started = false;

/** Load every device's look once and follow the changes. */
function start() {
  if (started) return;
  started = true;
  commands.wallpapers().then(
    (all) => setLooks(reconcile({ ...all, [OWN]: looks[OWN] })),
    () => {},
  );
  void listen<{ node: string; wallpaper: Wallpaper_Serialize | null }>("wallpaper-changed", (e) =>
    setLooks(e.payload.node, e.payload.wallpaper ?? undefined),
  );
}

/** This computer's own look (asked again each time a tile for it shows:
 *  it changes without an event). */
function loadOwn() {
  commands.ownWallpaper().then(
    (w) => setLooks(OWN, w ?? undefined),
    () => {},
  );
}

/** The look `node` shared (none given: this computer's). */
export function wallpaperOf(node?: string): Wallpaper_Serialize | undefined {
  return looks[node ?? OWN];
}

// --- shared colours ----------------------------------------------------------------

const isHex = (c: string) => /^#[0-9a-f]{6}$/i.test(c.trim());

/** A smooth gradient from the colours: the main one across, the others
 *  as soft glows from two corners. */
export function gradientOf(colors: string[]): string {
  const c = colors.filter(isHex);
  const a = c[0] ?? "#1a2350";
  const b = c[1] ?? a;
  const d = c[2] ?? b;
  return [
    `radial-gradient(120% 90% at 15% 10%, ${b} 0%, transparent 62%)`,
    `radial-gradient(110% 90% at 90% 95%, ${d} 0%, transparent 60%)`,
    `linear-gradient(150deg, ${a}, ${b} 55%, ${d})`,
  ].join(", ");
}

// --- built-in wallpapers -----------------------------------------------------------

/** A built-in wallpaper: a night sky from `top` to `bottom`, three
 *  ribbons of light and a glow, turned by `turn` degrees. */
type Art = { top: string; bottom: string; ribbons: [string, string, string]; glow: string; turn: number };

export const DEFAULT_ARTS: Art[] = [
  // Blue waves.
  { top: "#040a26", bottom: "#0d1d55", ribbons: ["#2f6bff", "#7a4dff", "#3fc8ff"], glow: "#3d5bff", turn: -28 },
  // Violet aurora.
  { top: "#08051d", bottom: "#200b4a", ribbons: ["#9b5cff", "#ff5fa8", "#4f7dff"], glow: "#8a3dff", turn: 22 },
  // Teal aurora.
  { top: "#02121a", bottom: "#06283d", ribbons: ["#19d3c5", "#3a7bff", "#7cf2b0"], glow: "#12a7c9", turn: -12 },
  // Indigo drift.
  { top: "#060a1c", bottom: "#161b44", ribbons: ["#5e6bff", "#b06bff", "#2bd0ff"], glow: "#5b4dff", turn: 36 },
  // Dusk orbit.
  { top: "#0b0716", bottom: "#2a0f33", ribbons: ["#ff7a59", "#b84dff", "#ffb36b"], glow: "#c2477a", turn: -40 },
];

/** The same built-in wallpaper for the same device, different ones for
 *  different devices. */
export function artIndex(key: string): number {
  let h = 2166136261;
  for (let i = 0; i < key.length; i++) h = Math.imul(h ^ key.charCodeAt(i), 16777619);
  return (h >>> 0) % DEFAULT_ARTS.length;
}

/** A ribbon across the box: its top edge a wave from `y`, `thick` deep. */
function ribbon(y: number, amp: number, thick: number): string {
  const top = `M-20 ${y} C 20 ${y - amp} 60 ${y + amp} 120 ${y - amp / 2}`;
  const back = `L120 ${y - amp / 2 + thick} C 60 ${y + amp + thick * 1.6} 20 ${y - amp + thick * 0.4} -20 ${y + thick}`;
  return `${top} ${back} Z`;
}

/** A few faint stars, the same for the same seed. */
function stars(seed: number): { x: number; y: number; r: number }[] {
  let s = seed * 9301 + 49297;
  const next = () => ((s = (s * 9301 + 49297) % 233280) / 233280);
  return Array.from({ length: 14 }, () => ({ x: next() * 100, y: next() * 55, r: 0.25 + next() * 0.45 }));
}

export function DefaultArt(props: { index: number }) {
  const id = createUniqueId();
  const art = () => DEFAULT_ARTS[props.index] ?? DEFAULT_ARTS[0];
  return (
    <svg
      viewBox="0 0 100 100"
      preserveAspectRatio="xMidYMid slice"
      style={{ position: "absolute", inset: 0, width: "100%", height: "100%", display: "block" }}
      aria-hidden="true"
    >
      <defs>
        <linearGradient id={`${id}-sky`} x1="0" y1="0" x2="0" y2="1">
          <stop offset="0" stop-color={art().top} />
          <stop offset="1" stop-color={art().bottom} />
        </linearGradient>
        <radialGradient id={`${id}-glow`} cx="0.5" cy="0.55" r="0.55">
          <stop offset="0" stop-color={art().glow} stop-opacity="0.75" />
          <stop offset="1" stop-color={art().glow} stop-opacity="0" />
        </radialGradient>
        <For each={art().ribbons}>
          {(c, i) => (
            <linearGradient id={`${id}-r${i()}`} x1="0" y1="0" x2="1" y2="0">
              <stop offset="0" stop-color={c} stop-opacity="0" />
              <stop offset="0.45" stop-color={c} stop-opacity="0.9" />
              <stop offset="1" stop-color={c} stop-opacity="0.15" />
            </linearGradient>
          )}
        </For>
      </defs>
      <rect width="100" height="100" fill={`url(#${id}-sky)`} />
      <For each={stars(props.index + 1)}>{(s) => <circle cx={s.x} cy={s.y} r={s.r} fill="#fff" opacity="0.55" />}</For>
      <g transform={`rotate(${art().turn} 50 50)`}>
        <ellipse cx="50" cy="58" rx="60" ry="34" fill={`url(#${id}-glow)`} />
        <path d={ribbon(40, 16, 12)} fill={`url(#${id}-r0)`} />
        <path d={ribbon(55, 22, 8)} fill={`url(#${id}-r1)`} opacity="0.85" />
        <path d={ribbon(64, 12, 5)} fill={`url(#${id}-r2)`} opacity="0.9" />
        <path d={ribbon(50, 18, 1.2)} fill="#ffffff" opacity="0.35" />
      </g>
    </svg>
  );
}

// --- frames -------------------------------------------------------------------------

export type Frame = "phone" | "tablet" | "laptop" | "monitor";

/** The frame for a device icon name (`features::device_kind::ICONS`). */
export function frameOf(icon: string): Frame {
  if (/phone|foldable|watch/.test(icon)) return "phone";
  if (/tablet/.test(icon)) return "tablet";
  if (/laptop/.test(icon)) return "laptop";
  // Nothing known: a laptop, the most common other device for a computer.
  if (icon === "unknown") return "laptop";
  return "monitor";
}

/** Named sizes: the art's height in px. */
export const ART_SIZES = { small: 64, medium: 120, large: 160 } as const;

const BODY = "linear-gradient(150deg, #3a3f4c 0%, #151820 45%, #0a0c12 100%)";
const EDGE = "inset 0 0 0 1px rgba(255,255,255,0.16), 0 6px 18px rgba(0,0,0,0.35)";
const METAL = "linear-gradient(180deg, #d9dde5 0%, #a5abb8 55%, #6e7482 100%)";

export type DeviceArtProps = {
  /** The device (node id); none: this computer. */
  node?: string;
  /** Its device icon name (`peerIcon(node)`, or this computer's): picks
   *  the frame (phone, tablet, laptop, monitor). */
  icon: string;
  /** "small" (list rows, 64 px tall), "medium" (cards, 120), "large"
   *  (the device page, 160), or a height in px. Default "medium". */
  size?: keyof typeof ART_SIZES | number;
  /** The screen when it shared nothing (default: its built-in wallpaper). */
  fallback?: JSX.Element;
  /** Read out for the picture (its name); none: decorative. */
  title?: string;
  class?: string;
};

export default function DeviceArt(props: DeviceArtProps) {
  start();
  onMount(() => {
    if (props.node === undefined) loadOwn();
  });
  const h = () => (typeof props.size === "number" ? props.size : ART_SIZES[props.size ?? "medium"]);
  const frame = () => frameOf(props.icon);
  const look = () => wallpaperOf(props.node);
  const colors = () => (look()?.colors ?? []).filter(isHex);
  const label = (): JSX.HTMLAttributes<HTMLDivElement> =>
    props.title ? { role: "img", "aria-label": props.title } : { "aria-hidden": true };

  /** What the screen shows, filling its box. */
  const screen = (radius: number): JSX.Element => (
    <div
      style={{
        position: "absolute",
        inset: 0,
        overflow: "hidden",
        "border-radius": `${radius}px`,
        background: colors().length ? gradientOf(colors()) : "#05081a",
      }}
    >
      <Show
        when={look()?.image}
        fallback={
          <Show when={!colors().length}>
            {props.fallback ?? <DefaultArt index={artIndex(props.node ?? `own:${props.icon}`)} />}
          </Show>
        }
      >
        {(src) => (
          <img
            src={src()}
            alt=""
            draggable={false}
            style={{ position: "absolute", inset: 0, width: "100%", height: "100%", "object-fit": "cover" }}
          />
        )}
      </Show>
      {/* Glass: a faint sheen across the screen. */}
      <span
        style={{
          position: "absolute",
          inset: 0,
          background: "linear-gradient(125deg, rgba(255,255,255,0.14) 0%, rgba(255,255,255,0) 38%)",
          "pointer-events": "none",
        }}
      />
    </div>
  );

  const box = (w: number, height: number): JSX.CSSProperties => ({
    position: "relative",
    width: `${Math.round(w)}px`,
    height: `${Math.round(height)}px`,
    "flex-shrink": 0,
  });

  /** The frame with its screen, sized for `size`. */
  const drawn = (): JSX.Element => {
    const H = h();
    switch (frame()) {
      case "phone":
      case "tablet": {
        const phone = frame() === "phone";
        const w = H * (phone ? 0.49 : 0.76);
        const bezel = Math.max(2, H * (phone ? 0.028 : 0.045));
        const r = H * (phone ? 0.11 : 0.06);
        const hole = Math.max(2, H * 0.03);
        return (
          <div style={{ ...box(w, H), background: BODY, "border-radius": `${r}px`, "box-shadow": EDGE }}>
            <div style={{ position: "absolute", inset: `${bezel}px` }}>{screen(Math.max(1, r - bezel))}</div>
            {/* The camera: a punch-hole on a phone, a dot in the bezel on a tablet. */}
            <span
              style={{
                position: "absolute",
                left: "50%",
                top: phone ? `${bezel + hole * 0.9}px` : `${(bezel - hole * 0.6) / 2}px`,
                width: `${phone ? hole : hole * 0.6}px`,
                height: `${phone ? hole : hole * 0.6}px`,
                transform: "translateX(-50%)",
                "border-radius": "50%",
                background: "#05060a",
                "box-shadow": "0 0 0 1px rgba(255,255,255,0.12)",
              }}
            />
          </div>
        );
      }
      case "laptop": {
        const lidH = H * 0.88;
        const lidW = lidH * 1.55;
        const baseH = Math.max(3, H * 0.07);
        const baseW = lidW * 1.16;
        const bezel = Math.max(2, H * 0.035);
        const r = Math.max(2, H * 0.045);
        return (
          <div style={box(baseW, H)}>
            <div
              style={{
                position: "absolute",
                left: `${(baseW - lidW) / 2}px`,
                top: 0,
                width: `${lidW}px`,
                height: `${lidH}px`,
                background: BODY,
                "border-radius": `${r}px ${r}px 2px 2px`,
                "box-shadow": EDGE,
              }}
            >
              <div style={{ position: "absolute", left: `${bezel}px`, right: `${bezel}px`, top: `${bezel}px`, bottom: `${bezel * 1.2}px` }}>
                {screen(Math.max(1, r - bezel))}
              </div>
            </div>
            {/* The base, with the notch to open it. */}
            <div
              style={{
                position: "absolute",
                left: 0,
                bottom: `${H - lidH - baseH}px`,
                width: `${baseW}px`,
                height: `${baseH}px`,
                background: METAL,
                "border-radius": `1px 1px ${baseH * 0.9}px ${baseH * 0.9}px`,
                "box-shadow": "0 4px 10px rgba(0,0,0,0.35)",
              }}
            >
              <span
                style={{
                  position: "absolute",
                  left: "50%",
                  top: 0,
                  width: `${baseW * 0.14}px`,
                  height: `${Math.max(1, baseH * 0.35)}px`,
                  transform: "translateX(-50%)",
                  background: "rgba(0,0,0,0.22)",
                  "border-radius": `0 0 ${baseH}px ${baseH}px`,
                }}
              />
            </div>
          </div>
        );
      }
      default: {
        const scrH = H * 0.74;
        const scrW = scrH * 1.6;
        const neckH = H * 0.19;
        const footH = Math.max(2, H * 0.07);
        const bezel = Math.max(2, H * 0.03);
        const r = Math.max(2, H * 0.035);
        return (
          <div style={box(scrW, H)}>
            {/* The stand: a neck and a foot. */}
            <div
              style={{
                position: "absolute",
                left: `${scrW * 0.44}px`,
                top: `${scrH - 2}px`,
                width: `${scrW * 0.12}px`,
                height: `${neckH + 2}px`,
                background: METAL,
                "clip-path": "polygon(12% 0, 88% 0, 100% 100%, 0 100%)",
              }}
            />
            <div
              style={{
                position: "absolute",
                left: `${scrW * 0.32}px`,
                bottom: 0,
                width: `${scrW * 0.36}px`,
                height: `${footH}px`,
                background: METAL,
                "border-radius": `${footH}px ${footH}px 2px 2px`,
                "box-shadow": "0 3px 8px rgba(0,0,0,0.35)",
              }}
            />
            <div
              style={{
                position: "absolute",
                inset: `0 0 auto 0`,
                height: `${scrH}px`,
                background: BODY,
                "border-radius": `${r}px`,
                "box-shadow": EDGE,
              }}
            >
              <div style={{ position: "absolute", inset: `${bezel}px` }}>{screen(Math.max(1, r - bezel))}</div>
            </div>
          </div>
        );
      }
    }
  };

  return (
    <div class={props.class} style={{ display: "inline-grid", "place-items": "center" }} {...label()}>
      {drawn()}
    </div>
  );
}

// --- the setting ---------------------------------------------------------------

/** "Share my wallpaper with my devices" (on by default; backend
 *  `wallpaper.rs`). Off: nothing is sent, and your other devices forget
 *  the one they had. */
export function WallpaperShareSetting() {
  const [on, setOn] = createSignal<boolean>();
  onMount(() => commands.shareWallpaper().then(setOn, (e) => toast(friendlyError(e), "error")));
  const save = async (v: boolean) => {
    try {
      await commands.setShareWallpaper(v);
      setOn(v);
    } catch (e) {
      toast(friendlyError(e), "error");
    }
  };
  return (
    <>
      <label class="switch-row">
        <span>{t("wallpaper.setting")}</span>
        <input
          type="checkbox"
          role="switch"
          checked={on() ?? true}
          disabled={on() === undefined}
          onChange={(e) => save(e.currentTarget.checked)}
        />
      </label>
      <p class="muted small">{t("wallpaper.settingHint")}</p>
    </>
  );
}
