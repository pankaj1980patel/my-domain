// A device page's arrangement ("Edit page"): which tiles come first and which
// are hidden. Saved per kind of device, or for one device, by the Rust side
// (page_layout.rs), which also merges a saved layout with the tiles the page
// has now (a tile added later appears at its default place).

import type { JSX } from "solid-js";
import { state } from "./core";

/** The two layouts kept: one for phones, one for computers. */
export type PageKind = "phone" | "computer";

/** One tile a device's page can show. Adding a tile to the page is adding
 *  one of these to `TILES` in screens/Device.tsx. */
export type TileDef = {
  /** Kept in saved layouts: never rename one. */
  id: string;
  /** Its name in "Edit page". */
  title: (node: string) => string;
  /** Spans the page's width (as its tile does). */
  wide?: boolean;
  /** Only on these kinds of device's page (default: both). */
  kinds?: PageKind[];
  /** Whether the device offers it now (default: always). */
  available?: (node: string) => boolean;
  render: (node: string) => JSX.Element;
};

/** Which layout `node`'s page uses: its own report first (its status,
 *  what kind of device it said it is), then its icon; a computer when
 *  nothing says. */
export function pageKind(node: string): PageKind {
  const platform = state.status[node]?.platform;
  if (platform) return platform === "android" ? "phone" : "computer";
  const os = state.kinds[node]?.os;
  if (os) return os === "android" || os === "ios" ? "phone" : "computer";
  const icon = state.labels[node]?.icon || state.kinds[node]?.icon || "";
  return isHandheldIcon(icon) ? "phone" : "computer";
}

/** A phone, tablet, foldable or watch icon. */
export function isHandheldIcon(icon: string): boolean {
  return ["phone", "tablet", "watch", "android_phone", "android_tablet", "android_foldable"].includes(icon);
}

/** `order` with `id` moved onto `over`'s place (dropped there). */
export function moveOnto(order: string[], id: string, over: string): string[] {
  const from = order.indexOf(id);
  const to = order.indexOf(over);
  if (from < 0 || to < 0 || from === to) return order;
  const out = order.filter((x) => x !== id);
  // Moving down: after `over`; moving up: before it.
  const at = out.indexOf(over) + (from < to ? 1 : 0);
  out.splice(at, 0, id);
  return out;
}

/** `order` with `id` one shown tile earlier (`step` -1) or later (+1);
 *  hidden tiles are stepped over. Unchanged at either end. */
export function moveStep(order: string[], hidden: string[], id: string, step: -1 | 1): string[] {
  const shown = order.filter((x) => x === id || !hidden.includes(x));
  const i = shown.indexOf(id);
  const next = shown[i + step];
  if (i < 0 || next === undefined) return order;
  return moveOnto(order, id, next);
}
