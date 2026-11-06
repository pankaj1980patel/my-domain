// "Download on…": pick the device that downloads a link itself, straight
// from the web (lib/remoteDownload.ts). Only devices that say they can are
// listed. And the question asked here the first time a device asks this
// computer to download something: Allow, Always allow that device, or
// Don't allow (Escape). The file is saved in the downloads folder and
// never opened by itself.

import { createEffect, createMemo, createSignal, For, on, Show } from "solid-js";
import { Portal } from "solid-js/web";
import { peerName, state } from "../lib/core";
import {
  answerDownload,
  closeDownloadPicker,
  downloadTargets,
  linkIn,
  pickingLink,
  requestDownload,
  waitingAsks,
} from "../lib/remoteDownload";
import { friendlyError, humanSize } from "../lib/words";
import { DeviceAvatar } from "./DeviceLabel";
import { StatusPill, toast } from "./ui";
import { t } from "../i18n";
import { rememberFocus, trapTab } from "../lib/a11y";

export function DownloadOnPicker() {
  const [text, setText] = createSignal("");
  const [targets, setTargets] = createSignal<string[]>([]);
  const [busy, setBusy] = createSignal(false);
  let dialog: HTMLDivElement | undefined;
  let restore: (() => void) | undefined;

  createEffect(
    on(pickingLink, (link) => {
      if (link === null) return;
      restore ??= rememberFocus();
      setText(link);
      downloadTargets().then(setTargets, () => setTargets([]));
      requestAnimationFrame(() => dialog?.querySelector<HTMLInputElement>("input")?.focus());
    }),
  );

  const close = () => {
    closeDownloadPicker();
    restore?.();
    restore = undefined;
  };

  /** The devices that download links: connected ones first. */
  const devices = createMemo(() =>
    state.peers
      .filter((p) => targets().includes(p.node_id))
      .sort((a, b) => {
        const rank = (n: string) => (state.links[n] ? 0 : 1);
        return rank(a.node_id) - rank(b.node_id) || peerName(a.node_id).localeCompare(peerName(b.node_id));
      }),
  );
  const link = () => linkIn(text());

  const send = async (node: string) => {
    const url = link();
    if (!url || busy()) return;
    setBusy(true);
    try {
      await requestDownload(node, url);
      toast(t("download.asked", { device: peerName(node) }), "ok");
      close();
    } catch (e) {
      toast(friendlyError(e), "error");
    } finally {
      setBusy(false);
    }
  };

  return (
    <Show when={pickingLink() !== null}>
      <Portal>
        <div class="send-to-backdrop" onMouseDown={(e) => e.target === e.currentTarget && close()}>
          <div
            class="send-to"
            ref={dialog}
            role="dialog"
            aria-modal="true"
            aria-labelledby="download-on-title"
            onKeyDown={(e) => {
              if (e.key === "Escape") {
                e.preventDefault();
                close();
              } else trapTab(e, dialog);
            }}
          >
            <h2 id="download-on-title">{t("download.pickTitle")}</h2>
            <p class="muted small">{t("download.pickBody")}</p>
            <input
              type="url"
              placeholder="https://"
              aria-label={t("download.link")}
              value={text()}
              onInput={(e) => setText(e.currentTarget.value)}
            />
            <Show when={text().trim() && !link()}>
              <p class="small text-error">{t("download.notALink")}</p>
            </Show>
            <ul class="send-to-list" aria-label={t("sendTo.devices")}>
              <For each={devices()} fallback={<li class="muted small">{t("download.noDevices")}</li>}>
                {(p) => (
                  <li>
                    <button class="send-to-device" type="button" disabled={busy() || !link()} onClick={() => send(p.node_id)}>
                      <DeviceAvatar node={p.node_id} />
                      <span class="send-to-name">{peerName(p.node_id)}</span>
                      <StatusPill node={p.node_id} />
                    </button>
                  </li>
                )}
              </For>
            </ul>
            <p class="muted small">{t("download.publicOnly")}</p>
            <div class="row send-to-actions">
              <button class="btn btn-ghost" type="button" onClick={close}>
                {t("settings.cancel")}
              </button>
            </div>
          </div>
        </div>
      </Portal>
    </Show>
  );
}

export function DownloadAsk() {
  const ask = () => waitingAsks()[0];
  const answer = async (allow: boolean, always: boolean) => {
    const a = ask();
    if (!a) return;
    try {
      await answerDownload(a.id, allow, always);
    } catch (e) {
      toast(friendlyError(e), "error");
    }
  };
  return (
    <Show when={ask()} keyed>
      {(a) => (
        <Portal>
          <div class="send-to-backdrop">
            <div
              class="send-to"
              role="alertdialog"
              aria-modal="true"
              aria-labelledby="download-ask-title"
              aria-describedby="download-ask-body"
              onKeyDown={(e) => {
                if (e.key === "Escape") {
                  e.preventDefault();
                  answer(false, false);
                }
              }}
            >
              <h2 id="download-ask-title">{t("download.askTitle", { device: peerName(a.node_id) })}</h2>
              <div id="download-ask-body">
                <p>
                  <strong>{a.name}</strong>
                  <span class="muted small">
                    {" · "}
                    {a.size > 0 ? humanSize(a.size) : t("download.sizeUnknown")}
                  </span>
                </p>
                <p class="muted small">{t("download.askFrom", { host: a.host })}</p>
                <p class="muted small download-ask-link" title={a.url}>
                  {a.url}
                </p>
                <p class="muted small">{t("download.askNote")}</p>
              </div>
              <div class="row">
                <button class="btn btn-primary" type="button" ref={(el) => setTimeout(() => el.focus())} onClick={() => answer(true, false)}>
                  {t("download.allow")}
                </button>
                <button class="btn btn-soft" type="button" onClick={() => answer(true, true)}>
                  {t("download.alwaysAllow", { device: peerName(a.node_id) })}
                </button>
                <button class="btn btn-ghost" type="button" onClick={() => answer(false, false)}>
                  {t("download.dontAllow")}
                </button>
              </div>
            </div>
          </div>
        </Portal>
      )}
    </Show>
  );
}
