// Files dragged from Finder / Explorer onto a device's page: an overlay
// while they're over the window, and each dropped file is sent to that
// device (it shows up in the Files tile like any other transfer).

import { createSignal, onCleanup, onMount, Show } from "solid-js";
// Portal: the overlay lives on <body>, clear of the tiles' blur layers.
import { Portal } from "solid-js/web";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { peerName, sendFile } from "../../lib/core";
import { friendlyError, niceName } from "../../lib/words";
import { toast } from "../../components/ui";
import { t, tn } from "../../i18n";

/** The core refuses anything that isn't a plain file with this. */
const isNotAFile = (e: unknown) => String(e instanceof Error ? e.message : e).includes("not a file");

export default function DropToSend(props: { node: string }) {
  const [over, setOver] = createSignal(false);
  const name = () => niceName(peerName(props.node));

  /** Send each path; folders are skipped and counted. */
  const sendAll = async (paths: string[]) => {
    let sent = 0;
    let folders = 0;
    let failed: unknown = null;
    for (const p of paths) {
      try {
        await sendFile(props.node, p);
        sent++;
      } catch (e) {
        if (isNotAFile(e)) folders++;
        else failed ??= e;
      }
    }
    if (sent) toast(tn("files.started", sent, { name: name() }), "ok");
    if (folders) toast(t("drop.folders"), "info");
    if (failed) toast(friendlyError(failed), "error");
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
