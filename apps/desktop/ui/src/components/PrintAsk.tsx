// "Print <file> from <phone>?" (print.rs, `print-ask`): shown only with
// "Ask before printing" on. Print says yes; Don't print (or Escape) says
// no and the phone hears it was declined. Two minutes without an answer is
// a no (`print-ask-done` then takes the prompt away).

import { createSignal, onCleanup, onMount, Show } from "solid-js";
import { Portal } from "solid-js/web";
import { listen } from "@tauri-apps/api/event";
import { commands } from "../bindings";
import { peerName } from "../lib/core";
import { t } from "../i18n";

type Ask = { id: string; node_id: string; name: string; printer: string; copies: number };

export function PrintAsk() {
  const [ask, setAsk] = createSignal<Ask | null>(null);
  const offs: (() => void)[] = [];
  onMount(() => {
    listen<Ask>("print-ask", (e) => setAsk(e.payload))
      .then((u) => offs.push(u))
      .catch(() => {});
    listen<string>("print-ask-done", (e) => {
      if (ask()?.id === e.payload) setAsk(null);
    })
      .then((u) => offs.push(u))
      .catch(() => {});
  });
  onCleanup(() => offs.forEach((f) => f()));

  const answer = (yes: boolean) => {
    const a = ask();
    setAsk(null);
    if (a) commands.printAnswer(a.id, yes).catch(() => {});
  };

  return (
    <Show when={ask()}>
      {(a) => (
        <Portal>
          <div class="send-to-backdrop">
            <div
              class="send-to"
              role="alertdialog"
              aria-modal="true"
              aria-labelledby="print-ask-title"
              aria-describedby="print-ask-body"
              onKeyDown={(e) => {
                if (e.key === "Escape") {
                  e.preventDefault();
                  answer(false);
                }
              }}
            >
              <h2 id="print-ask-title">{t("print.askTitle", { name: a().name, device: peerName(a().node_id) })}</h2>
              <p id="print-ask-body" class="muted small">
                {t(a().copies > 1 ? "print.askBodyCopies" : "print.askBody", { printer: a().printer, n: a().copies })}
              </p>
              <div class="row">
                <button class="btn btn-primary" type="button" ref={(el) => setTimeout(() => el.focus())} onClick={() => answer(true)}>
                  {t("print.yes")}
                </button>
                <button class="btn btn-ghost" type="button" onClick={() => answer(false)}>
                  {t("print.no")}
                </button>
              </div>
            </div>
          </div>
        </Portal>
      )}
    </Show>
  );
}
