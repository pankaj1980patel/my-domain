// "How your devices are connected": the map's picture, laid out by the
// backend (links_map.rs) from the core's links, and kept fresh while the
// screen shows it: at once when a link comes up or goes down here or a
// device sends its own links (`links-map-changed`), and every few seconds
// for the round trips.

import { createSignal } from "solid-js";
import { listen } from "@tauri-apps/api/event";
import { commands, type MapView_Serialize } from "../bindings";

export type MapView = MapView_Serialize;
export type MapEdge = MapView["edges"][number];
export type MapDevice = MapView["map"]["devices"][number];

const [mapView, setMapView] = createSignal<MapView | null>(null);
export { mapView };

/** Read the map again. */
export async function loadMap(): Promise<void> {
  setMapView(await commands.linksMap());
}

/** Keep the map fresh until the returned function is called. */
export function watchMap(everyMs = 5000): () => void {
  let stopped = false;
  let soon: ReturnType<typeof setTimeout> | undefined;
  // A burst of events reads it once.
  const again = () => {
    if (stopped || soon) return;
    soon = setTimeout(() => {
      soon = undefined;
      if (!stopped) loadMap().catch(() => {});
    }, 150);
  };
  loadMap().catch(() => {});
  const timer = setInterval(again, everyMs);
  const offs = ["peer-connected", "peer-disconnected", "peers-updated", "links-map-changed", "labels-changed"].map((name) =>
    listen(name, again),
  );
  return () => {
    stopped = true;
    clearInterval(timer);
    if (soon) clearTimeout(soon);
    for (const off of offs) off.then((f) => f(), () => {});
  };
}

/** The line from this computer to `node`. */
export function myEdge(v: MapView, node: string): MapEdge | undefined {
  return v.edges.find((e) => e.mine && e.b === node);
}
