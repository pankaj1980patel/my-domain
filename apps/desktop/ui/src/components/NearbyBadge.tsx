// "Nearby · low latency" on an active audio, screen or camera view: the
// stream with `node` runs on a direct link on the local network
// (`features::nearby`), with its end-to-end latency ("12 ms (was 140 ms)").
// Follows the core's `nearby-media` events.

import { createSignal, onCleanup, onMount, Show } from "solid-js";
import { listen } from "@tauri-apps/api/event";
import { commands } from "../bindings";
import { latencyText, showBadge, type NearbyStatus } from "../lib/nearby";
import { t } from "../i18n";

export function NearbyBadge(props: { node: string }) {
  const [s, setS] = createSignal<NearbyStatus | null>(null);
  let unlisten: (() => void) | undefined;
  onMount(async () => {
    commands.nearbyStatus(props.node).then(setS, () => {});
    unlisten = await listen<NearbyStatus>("nearby-media", (e) => {
      if (e.payload.node_id === props.node) setS(e.payload);
    });
  });
  onCleanup(() => unlisten?.());
  return (
    <Show when={s()?.media}>
      <p class="nearby-badge-row small" aria-live="polite">
        <Show when={showBadge(s())}>
          <span class="nearby-badge" title={t("nearby.badgeHint")}>
            {t("nearby.badge")}
          </span>
        </Show>
        <Show when={latencyText(s())}>
          <span class="muted">{latencyText(s())}</span>
        </Show>
      </p>
    </Show>
  );
}
