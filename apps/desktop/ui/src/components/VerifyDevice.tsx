// "Verify" on a device's page: both security keys' fingerprints side by
// side, the safety number (the same on both devices) and a QR code the
// phone scans with its camera. When they match, "Mark as verified" (the
// phone marks itself when its scan checks out). A "Verified" badge shows on
// the page and the connection map until the device's key changes.

import { createEffect, createSignal, For, on, Show } from "solid-js";
import { Portal } from "solid-js/web";
import { peerName } from "../lib/core";
import { isVerified, loadVerification, ownFingerprint, safetyLines, setVerified, type KeyVerification } from "../lib/verify";
import { friendlyError } from "../lib/words";
import { rememberFocus, trapTab } from "../lib/a11y";
import { toast } from "./ui";
import { t } from "../i18n";

/** "Verified", for a device whose key you verified. */
export function VerifiedBadge(props: { verified: boolean }) {
  return (
    <Show when={props.verified}>
      <span class="verified-badge" title={t("verify.badgeHint")}>
        {t("verify.badge")}
      </span>
    </Show>
  );
}

/** This computer's own fingerprint (Settings, Add a device). */
export function OwnFingerprint() {
  return (
    <div class="own-fp">
      <p class="small">
        {t("verify.yourKey")} <code class="mono">{ownFingerprint() || "…"}</code>
      </p>
      <p class="muted small">{t("verify.yourKeyHint")}</p>
    </div>
  );
}

/** The badge and the "Verify" button on a device's page. */
export default function VerifyDevice(props: { node: string }) {
  const [info, setInfo] = createSignal<KeyVerification | null>(null);
  const [open, setOpen] = createSignal(false);
  createEffect(
    on(
      () => props.node,
      (node) => {
        setInfo(null);
        loadVerification(node)
          .then(setInfo)
          .catch(() => {});
      },
    ),
  );
  const show = () =>
    loadVerification(props.node)
      .then((v) => {
        setInfo(v);
        if (v) setOpen(true);
        else toast(t("verify.notYet", { name: peerName(props.node) }), "info");
      })
      .catch((e) => toast(friendlyError(e), "error"));
  return (
    <>
      <VerifiedBadge verified={isVerified(props.node)} />
      <button class="btn btn-ghost btn-sm" title={t("verify.buttonHint")} onClick={show}>
        {t("verify.button")}
      </button>
      <Show when={open() && info()}>
        {(v) => <VerifyDialog info={v()} onChange={setInfo} onClose={() => setOpen(false)} />}
      </Show>
    </>
  );
}

function VerifyDialog(props: { info: KeyVerification; onChange: (v: KeyVerification | null) => void; onClose: () => void }) {
  let dialog: HTMLDivElement | undefined;
  const restore = rememberFocus();
  const close = () => {
    props.onClose();
    restore();
  };
  const name = () => peerName(props.info.node_id);
  const side = () => props.info.qr_size + 8;
  const mark = (on: boolean) =>
    setVerified(props.info.node_id, on)
      .then(() => {
        toast(on ? t("verify.done", { name: name() }) : t("verify.undone", { name: name() }), "ok");
        return loadVerification(props.info.node_id).then(props.onChange);
      })
      .catch((e) => toast(t("verify.failed", { why: friendlyError(e) }), "error"));
  return (
    <Portal>
      <div class="clip-pic-backdrop" onMouseDown={(e) => e.target === e.currentTarget && close()}>
        <div
          class="clip-pic-preview verify-dialog"
          ref={(el) => {
            dialog = el;
            queueMicrotask(() => el.querySelector<HTMLElement>("button")?.focus());
          }}
          role="dialog"
          aria-modal="true"
          aria-labelledby="verify-title"
          onKeyDown={(e) => {
            if (e.key === "Escape") {
              e.preventDefault();
              close();
            } else trapTab(e, dialog);
          }}
        >
          <div class="tile-head">
            <h2 id="verify-title">{t("verify.title", { name: name() })}</h2>
            <button class="btn btn-ghost btn-sm" onClick={close}>
              {t("verify.close")}
            </button>
          </div>
          <p class="small">{t("verify.intro", { name: name() })}</p>
          <Show when={props.info.key_changed}>
            <p class="small warn-text">{t("verify.keyChangedFirst", { name: name() })}</p>
          </Show>
          <div class="verify-fps">
            <div>
              <span class="muted small">{t("verify.thisComputer")}</span>
              <code class="mono">{props.info.own_fingerprint}</code>
            </div>
            <div>
              <span class="muted small">{name()}</span>
              <code class="mono">{props.info.their_fingerprint}</code>
            </div>
          </div>
          <div>
            <span class="muted small">{t("verify.safetyNumber")}</span>
            <div class="safety-number mono" aria-label={t("verify.safetyNumber")}>
              <For each={safetyLines(props.info.safety_number)}>{(line) => <div>{line}</div>}</For>
            </div>
          </div>
          <Show when={props.info.qr_path}>
            <svg
              class="join-qr verify-qr"
              viewBox={`0 0 ${side()} ${side()}`}
              role="img"
              aria-label={t("verify.qrLabel")}
              shape-rendering="crispEdges"
            >
              <rect width={side()} height={side()} fill="#fff" />
              <path d={props.info.qr_path} fill="#000" />
            </svg>
            <p class="muted small">{t("verify.scan", { name: name() })}</p>
          </Show>
          <p class="small">
            <VerifiedBadge verified={props.info.verified} />{" "}
            {props.info.verified ? t("verify.isVerified", { name: name() }) : t("verify.notVerified")}
          </p>
          <div class="row">
            <Show
              when={props.info.verified}
              fallback={
                <button class="btn btn-primary btn-sm" disabled={props.info.key_changed} onClick={() => mark(true)}>
                  {t("verify.match")}
                </button>
              }
            >
              <button class="btn btn-ghost btn-sm" onClick={() => mark(false)}>
                {t("verify.unmark")}
              </button>
            </Show>
          </div>
          <p class="muted small">{t("verify.mismatch")}</p>
        </div>
      </div>
    </Portal>
  );
}
