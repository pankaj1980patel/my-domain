// "Quick actions": big tiles for one device (send a file, send the
// clipboard, its screen, a call, ring, find). An action it can't do at all
// isn't shown; one it can't do right now is greyed, with the reason as its
// tooltip and in words for screen readers.

import { For, Show } from "solid-js";
import { peerName, runAction, state } from "../lib/core";
import { friendlyError } from "../lib/words";
import { quickRing, quickSendClipboard, quickSendFile } from "../lib/quickActions";
import { Icon, type IconName, type Tone } from "./Icon";
import { toast } from "./ui";
import { t } from "../i18n";

/** `hidden`: the device can't do it at all (no Call on a laptop): not
 *  shown. `off`: it can, but not right now (offline, location sharing off):
 *  greyed, with the reason. */
type Action = { icon: IconName; tone: Tone; label: string; run: () => unknown; off?: string; hidden?: boolean };

export function QuickGrid(props: {
  node: string;
  vertical?: boolean;
  onGo: (tab: string, tile?: string) => void;
  /** The Overview's row: "Create note" in place of Ring and Find. */
  onNote?: () => void;
}) {
  const st = () => state.status[props.node];
  const name = () => peerName(props.node);
  const has = (a: string) => !!st()?.actions.includes(a);
  const live = () => !!state.links[props.node];
  const offline = () => (live() ? undefined : t("palette.offline", { name: name() }));
  const locate = () =>
    runAction(props.node, "locate", {})
      .then(() => toast(t("locate.asking")))
      .catch((e) => toast(friendlyError(e), "error"));
  // Tones by kind: files and screens blue/cyan, calls green, ring amber,
  // find violet (red is kept for danger).
  const all = (): Action[] => [
    { icon: "send-file", tone: "blue", label: t("quick.sendFile"), run: () => quickSendFile(props.node) },
    { icon: "send-clip", tone: "cyan", label: t("quick.sendClipboard"), run: () => quickSendClipboard(props.node) },
    { icon: "screen", tone: "cyan", label: t("quick.viewScreen"), run: () => props.onGo("remote", "screen"), hidden: !has("screen"), off: offline() },
    { icon: "call", tone: "green", label: t("control.dial"), run: () => props.onGo("remote", "control"), hidden: !has("dial"), off: offline() },
    { icon: "ring", tone: "amber", label: t("control.ring"), run: () => quickRing(props.node) },
    {
      icon: "find",
      tone: "violet",
      label: t("quick.find"),
      run: locate,
      hidden: !has("locate"),
      off: offline() ?? (st()?.locationSharing ? undefined : t("locate.offOn", { name: name() })),
    },
  ];
  const actions = (): Action[] => {
    const list = all().filter((a) => !a.hidden);
    if (!props.onNote) return list;
    const note = props.onNote;
    // The Overview's row: the device's first four, then Create note.
    return [...list.filter((a) => a.icon !== "ring" && a.icon !== "find").slice(0, 4), { icon: "notes", tone: "amber", label: t("quick.note"), run: note }];
  };
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
