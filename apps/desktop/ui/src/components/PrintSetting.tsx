// "Print from your phone" (print.rs): "Let my phone print here" (on by
// default) lists this computer and its printers in the phone's print
// dialog; "Ask before printing" (off by default) asks here first. macOS
// and Linux print through CUPS; Windows can't print a PDF by itself, so
// the tile says so there.

import { createSignal, For, onMount, Show } from "solid-js";
import { commands, type PrintSetting as Setting } from "../bindings";
import { friendlyError } from "../lib/words";
import { toast } from "./ui";
import { t } from "../i18n";
import { HelpLink } from "../help/HelpLink";

export function PrintSetting() {
  const [s, setS] = createSignal<Setting>();
  const [busy, setBusy] = createSignal(false);
  const run = async (f: () => Promise<Setting>) => {
    setBusy(true);
    try {
      setS(await f());
    } catch (e) {
      toast(friendlyError(e), "error");
    } finally {
      setBusy(false);
    }
  };
  onMount(() => run(() => commands.printSetting()));

  return (
    <article class="tile" id="settings-print">
      <h2>
        {t("print.title")}
        <HelpLink topic="print" />
      </h2>
      <Show when={s()} fallback={<p class="muted small">{t("print.loading")}</p>}>
        <Show when={s()?.supported} fallback={<p class="muted small">{t("print.unsupported")}</p>}>
          <label class="switch-row">
            <span>{t("print.allow")}</span>
            <input
              type="checkbox"
              role="switch"
              checked={s()?.allow ?? true}
              disabled={busy()}
              onChange={(e) => {
                const on = e.currentTarget.checked;
                run(() => commands.setPrintAllow(on));
              }}
            />
          </label>
          <p class="muted small">{t("print.allowHint")}</p>
          <Show when={s()?.allow}>
            <label class="switch-row">
              <span>{t("print.ask")}</span>
              <input
                type="checkbox"
                role="switch"
                checked={s()?.ask ?? false}
                disabled={busy()}
                onChange={(e) => {
                  const on = e.currentTarget.checked;
                  run(() => commands.setPrintAsk(on));
                }}
              />
            </label>
            <p class="muted small">{t("print.askHint")}</p>
            <h3 class="small">{t("print.printers")}</h3>
            <Show when={(s()?.printers ?? []).length > 0} fallback={<p class="muted small">{t("print.none")}</p>}>
              <ul class="plain small">
                <For each={s()?.printers ?? []}>
                  {(p) => (
                    <li>
                      {p.label || p.name}
                      <Show when={p.is_default}>
                        {" "}
                        <span class="muted">({t("print.default")})</span>
                      </Show>
                    </li>
                  )}
                </For>
              </ul>
            </Show>
            <button class="btn btn-ghost" type="button" disabled={busy()} onClick={() => run(() => commands.printSetting())}>
              {t("print.refresh")}
            </button>
          </Show>
        </Show>
      </Show>
    </article>
  );
}
