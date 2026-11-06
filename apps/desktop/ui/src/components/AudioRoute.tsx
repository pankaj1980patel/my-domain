// Audio with a device, which way: "Auto" (Wi-Fi or the internet first,
// Bluetooth when there's no network link), "Wi-Fi only" or "Bluetooth only"
// (mdcore `features::audio_share`, kept per device), and a "via Bluetooth"
// badge while the stream goes over Bluetooth. Shown on both audio tiles.

import { createSignal, onCleanup, onMount, Show } from "solid-js";
import { commands } from "../bindings";
import { t } from "../i18n";

export const ROUTES = ["auto", "wifi", "bluetooth"] as const;
export type Route = (typeof ROUTES)[number];

/** A saved route, or Auto for anything else. */
export function routeOf(v: string | null | undefined): Route {
  return (ROUTES as readonly string[]).includes(v ?? "") ? (v as Route) : "auto";
}

export function AudioRouteBadge(props: { node: string }) {
  const [via, setVia] = createSignal("");
  let poll: ReturnType<typeof setInterval> | undefined;
  onMount(() => {
    const check = () => commands.audioVia(props.node).then(setVia, () => {});
    check();
    poll = setInterval(check, 1000);
  });
  onCleanup(() => poll && clearInterval(poll));
  return (
    <Show when={via() === "bluetooth"}>
      <p class="nearby-badge-row small" aria-live="polite">
        <span class="nearby-badge audio-ble-badge" title={t("audioRoute.badgeHint")}>
          {t("audioRoute.badge")}
        </span>
      </p>
    </Show>
  );
}

export function AudioRoutePicker(props: { node: string }) {
  const [route, setRoute] = createSignal<Route>("auto");
  onMount(() => {
    commands.audioRoute(props.node).then((r) => setRoute(routeOf(r)), () => {});
  });
  const choose = (v: string) => {
    const r = routeOf(v);
    setRoute(r);
    commands.setAudioRoute(props.node, r);
  };
  return (
    <label class="row small">
      <span class="muted">{t("audioRoute.label")}</span>
      <select aria-label={t("audioRoute.label")} onChange={(e) => choose(e.currentTarget.value)}>
        <option value="auto" selected={route() === "auto"}>
          {t("audioRoute.auto")}
        </option>
        <option value="wifi" selected={route() === "wifi"}>
          {t("audioRoute.wifi")}
        </option>
        <option value="bluetooth" selected={route() === "bluetooth"}>
          {t("audioRoute.bluetooth")}
        </option>
      </select>
    </label>
  );
}
