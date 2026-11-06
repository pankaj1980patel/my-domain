// Files dragged from Finder / Explorer onto a device's page: an overlay
// while they're over the window, and each dropped file is sent to that
// device (it shows up in the Files tile like any other transfer). Several
// files, or a folder (every file inside, recreated there as a folder), go
// as one batch.

import { createSignal, onCleanup, onMount, Show } from "solid-js";
// Portal: the overlay lives on <body>, clear of the tiles' blur layers.
import { Portal } from "solid-js/web";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { peerName, sendPaths } from "../../lib/core";
import { friendlyError } from "../../lib/words";
import { toast } from "../../components/ui";
import { t, tn } from "../../i18n";

export default function DropToSend(props: { node: string }) {
  const [over, setOver] = createSignal(false);
  const name = () => peerName(props.node);

  /** One file on its own; several, or a folder with everything in it, as
   *  one batch (one row in Transfers). */
  const sendAll = async (paths: string[]) => {
    try {
      const sent = await sendPaths(props.node, paths);
      if (sent) toast(tn("files.started", sent, { name: name() }), "ok");
    } catch (e) {
      toast(friendlyError(e), "error");
    }
  };

  // Listens only while the device page is shown. The listener arrives
  // after an await, so cleanup is registered up front and handles both
  // orders (page closed before or after it's ready).
  let gone = false;
  let unlisten: (() => void) | undefined;
  onCleanup(() => {
    gone = true;
    unlisten?.();
  });
  onMount(async () => {
    const stop = await getCurrentWebview().onDragDropEvent((ev) => {
      const p = ev.payload;
      if (p.type === "enter" || p.type === "over") setOver(true);
      else if (p.type === "leave") setOver(false);
      else if (p.type === "drop") {
        setOver(false);
        if (p.paths.length) sendAll(p.paths);
      }
    });
    if (gone) stop();
    else unlisten = stop;
  });

  return (
    <Show when={over()}>
      <Portal>
        <div class="drop-overlay" role="status" aria-live="polite">
          <div class="drop-card">
            <strong>{t("drop.here", { name: name() })}</strong>
            <span class="muted small">{t("drop.hint")}</span>
          </div>
        </div>
      </Portal>
    </Show>
  );
}
