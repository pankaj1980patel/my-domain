// "Wake" (the hero's lead button) on another computer's page while it's offline and said it may
// be woken (Wake-on-LAN, mdcore features::wake): sends the magic packet from
// here and from your other devices on its network, then shows "Waking…"
// (trying to connect every 10 s) until it connects or a minute passes.

import { createEffect, createSignal, on, onCleanup, Show } from "solid-js";
import { commands } from "../../bindings";
import { connect, peerName, state, unwrap } from "../../lib/core";
import { friendlyError } from "../../lib/words";
import { linkStateOf, toast } from "../../components/ui";
import { t } from "../../i18n";
import { Icon } from "../../components/Icon";

/** How long "Waking…" waits for it to connect. */
const WAIT_MS = 60_000;
/** How often it tries to connect meanwhile. */
const RETRY_MS = 10_000;

export default function WakeButton(props: { node: string }) {
  const [can, setCan] = createSignal(false);
  const [waking, setWaking] = createSignal(false);
  const s = () => linkStateOf(props.node);
  const asleep = () => s() === "offline" || s() === "asleep" || s() === "failed";
  // Known across restarts; learnt from its status, so asked again when one comes.
  createEffect(
    on(
      () => [props.node, state.status[props.node]] as const,
      ([node]) => {
        commands.canWake(node).then(setCan, () => setCan(false));
      },
    ),
  );

  let retry: ReturnType<typeof setInterval> | undefined;
  let giveUp: ReturnType<typeof setTimeout> | undefined;
  const stop = () => {
    clearInterval(retry);
    clearTimeout(giveUp);
    retry = undefined;
    giveUp = undefined;
    setWaking(false);
  };
  onCleanup(stop);
  // Another device's page: whatever was waking stops showing.
  createEffect(on(() => props.node, stop, { defer: true }));
  // It connected: awake.
  createEffect(() => {
    if (waking() && (s() === "direct" || s() === "relay")) {
      stop();
      toast(t("wake.awake", { name: peerName(props.node) }), "ok");
    }
  });

  const wake = async () => {
    const node = props.node;
    try {
      const r = await unwrap(commands.wakeDevice(node));
      toast(
        r.relays.length > 0
          ? t("wake.sentRelayed", { names: r.relays.map((n) => peerName(n)).join(", ") })
          : t("wake.sent"),
        "ok",
      );
    } catch (e) {
      toast(friendlyError(e), "error");
      return;
    }
    stop();
    setWaking(true);
    retry = setInterval(() => {
      const now = linkStateOf(node);
      if (now !== "connecting" && now !== "waking") connect(node).catch(() => {});
    }, RETRY_MS);
    giveUp = setTimeout(() => {
      stop();
      toast(t("wake.noAnswer", { name: peerName(node) }), "error");
    }, WAIT_MS);
  };

  return (
    <Show when={can() && (asleep() || waking())}>
      <button class="btn btn-primary wake-btn" disabled={waking()} aria-busy={waking()} title={t("wake.hint")} onClick={wake}>
        <Show when={waking()} fallback={<Icon name="power" />}>
          <span class="wake-spin" aria-hidden="true" />
        </Show>
        {waking() ? t("wake.waking") : t("wake.button")}
      </button>
    </Show>
  );
}
