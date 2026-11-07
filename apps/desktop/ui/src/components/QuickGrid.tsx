// "Quick actions": six big tiles for one device (send a file, send the
// clipboard, its screen, a call, ring, find). An action it doesn't offer
// stays in place, greyed, with the reason as its tooltip and in words for
// screen readers.

import { For, Show } from "solid-js";
import { peerName, runAction, state } from "../lib/core";
import { friendlyError } from "../lib/words";
import { quickRing, quickSendClipboard, quickSendFile } from "../lib/quickActions";
import { Icon, type IconName, type Tone } from "./Icon";
import { toast } from "./ui";
import { t } from "../i18n";

type Action = { icon: IconName; tone: Tone; label: string; run: () => unknown; off?: string };

export function QuickGrid(props: { node: string; vertical?: boolean; onGo: (tab: string, tile?: string) => void }) {
  const st = () => state.status[props.node];
  const name = () => peerName(props.node);
  const has = (a: string) => !!st()?.actions.includes(a);
  const locate = () =>
    runAction(props.node, "locate", {})
      .then(() => toast(t("locate.asking")))
      .catch((e) => toast(friendlyError(e), "error"));
  const actions = (): Action[] => [
    { icon: "send-file", tone: "blue", label: t("quick.sendFile"), run: () => quickSendFile(props.node) },
    { icon: "send-clip", tone: "cyan", label: t("quick.sendClipboard"), run: () => quickSendClipboard(props.node) },
    {
      icon: "screen",
      tone: "blue",
      label: t("quick.viewScreen"),
      run: () => props.onGo("remote", "screen"),
      off: has("screen") ? undefined : t("quick.noScreen", { name: name() }),
    },
    {
      icon: "call",
      tone: "green",
      label: t("control.dial"),
      run: () => props.onGo("remote", "control"),
      off: has("dial") ? undefined : t("quick.noCall", { name: name() }),
    },
    { icon: "ring", tone: "amber", label: t("control.ring"), run: () => quickRing(props.node) },
    {
      icon: "find",
      tone: "red",
      label: t("quick.find"),
      run: locate,
      off: !has("locate")
        ? t("quick.noFind", { name: name() })
        : st()?.locationSharing
          ? undefined
          : t("locate.offOn", { name: name() }),
    },
  ];
  return (
    <div class="quick-grid" classList={{ "quick-list": !!props.vertical }}>
      <For each={actions()}>
        {(a) => (
          <button class="quick-tile" disabled={!!a.off} title={a.off ?? a.label} onClick={() => a.run()}>
            <Icon name={a.icon} tone={a.off ? undefined : a.tone} size={24} />
            <span>{a.label}</span>
            <Show when={a.off}>
              <span class="sr-only">{t("a11y.unavailable", { reason: a.off ?? "" })}</span>
            </Show>
          </button>
        )}
      </For>
    </div>
  );
}
