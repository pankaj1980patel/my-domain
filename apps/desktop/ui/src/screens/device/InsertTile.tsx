// Insert from phone (backend `insert_capture.rs`, mdcore
// `features::insert_capture`), like Continuity Camera: "Take a photo" or
// "Scan documents" asks the phone; it opens its camera once you act on the
// phone, and what it sends lands in Downloads and on the clipboard (a toast
// with Show in folder, from `core.ts`). While waiting: what the phone is
// doing, and Cancel. The phone gives up after 3 minutes.

import { Show } from "solid-js";
import { askInsert, cancelInsert, peerName, state } from "../../lib/core";
import { t } from "../../i18n";
import { HelpLink } from "../../help/HelpLink";

export default function InsertTile(props: { node: string }) {
  const name = () => peerName(props.node);
  const ask = () => state.inserts[props.node];
  const waiting = () => {
    const a = ask();
    if (!a) return "";
    if (a.phase === "sending") return t("insert.sending", { device: name() });
    if (a.phase === "open") return t(a.mode === "scan" ? "insert.scanning" : "insert.taking", { device: name() });
    return t("insert.waiting", { device: name() });
  };

  return (
    <article class="tile">
      <h2>
        {t("insert.title")}
        <HelpLink topic="screen" />
      </h2>
      <p class="muted small">{t("insert.hint", { device: name() })}</p>
      <Show
        when={ask()}
        fallback={
          <div class="row">
            <button class="btn btn-soft btn-sm" onClick={() => void askInsert(props.node, "photo")}>
              {t("insert.photo")}
            </button>
            <button class="btn btn-soft btn-sm" onClick={() => void askInsert(props.node, "scan")}>
              {t("insert.scan")}
            </button>
          </div>
        }
      >
        <p class="small" role="status" aria-live="polite">
          {waiting()}
        </p>
        <div class="row">
          <button class="btn btn-ghost btn-sm" onClick={() => cancelInsert(props.node)}>
            {t("insert.cancel")}
          </button>
        </div>
      </Show>
    </article>
  );
}
