// "<name>'s security key changed": on a device's page (until you decide or
// say "Not now") and in the connection map's details (always, while it's
// refused). "Trust the new key" asks first, showing the old and new keys'
// short fingerprints; only the key shown is trusted. It says whether the old
// key had been verified, and to compare the new fingerprint with the one the
// other device shows in its own Settings.

import { createSignal, Show } from "solid-js";
import { Portal } from "solid-js/web";
import { peerName } from "../lib/core";
import { dismissKeyChange, keyChangeOf, trustNewKey, watchKeyChanges, type KeyChange } from "../lib/keyChanges";
import { friendlyError } from "../lib/words";
import { rememberFocus, trapTab } from "../lib/a11y";
import { toast } from "./ui";
import { t } from "../i18n";

export default function KeyChangeBanner(props: { node: string; showDismissed?: boolean }) {
  watchKeyChanges();
  const [asking, setAsking] = createSignal<KeyChange | null>(null);
  const change = () => {
    const c = keyChangeOf(props.node);
    return c && (props.showDismissed || !c.dismissed) ? c : undefined;
  };
  const name = () => peerName(props.node);
  return (
    <Show when={change()}>
      {(c) => (
        <div class="key-banner" role="alert">
          <strong>{t("keyChange.title", { name: name() })}</strong>
          <Show when={c().was_verified}>
            <p class="small key-was-verified">{t("keyChange.wasVerified")}</p>
          </Show>
          <p class="small">{t("keyChange.body")}</p>
          <p class="small muted">
            {c().dismissed ? t("keyChange.dismissed", { name: name() }) : t("keyChange.refused", { name: name() })}
          </p>
          <div class="row">
            <button class="btn btn-soft btn-sm" onClick={() => setAsking(c())}>
              {t("keyChange.trust")}
            </button>
            <Show when={!c().dismissed}>
              <button
                class="btn btn-ghost btn-sm"
                onClick={() => dismissKeyChange(props.node).catch((e) => toast(friendlyError(e), "error"))}
              >
                {t("keyChange.notNow")}
              </button>
            </Show>
          </div>
          <Show when={asking()}>
            {(a) => <Confirm change={a()} name={name()} onClose={() => setAsking(null)} />}
          </Show>
        </div>
      )}
    </Show>
  );
}

function Confirm(props: { change: KeyChange; name: string; onClose: () => void }) {
  let dialog: HTMLDivElement | undefined;
  const restore = rememberFocus();
  const close = () => {
    props.onClose();
    restore();
  };
  const trust = () => {
    const name = props.name;
    trustNewKey(props.change.node_id, props.change.new_fingerprint)
      .then(() => toast(t("keyChange.trusted", { name }), "ok"))
      .catch((e) => toast(t("keyChange.failed", { why: friendlyError(e) }), "error"))
      .finally(close);
  };
  return (
    <Portal>
      <div class="clip-pic-backdrop" onMouseDown={(e) => e.target === e.currentTarget && close()}>
        <div
          class="clip-pic-preview key-confirm"
          ref={(el) => {
            dialog = el;
            // Cancel first: trusting is never the default.
            queueMicrotask(() => el.querySelector<HTMLElement>("button")?.focus());
          }}
          role="alertdialog"
          aria-modal="true"
          aria-labelledby="key-confirm-title"
          aria-describedby="key-confirm-body"
          onKeyDown={(e) => {
            if (e.key === "Escape") {
              e.preventDefault();
              close();
            } else trapTab(e, dialog);
          }}
        >
          <h2 id="key-confirm-title">{t("keyChange.confirmTitle", { name: props.name })}</h2>
          <p id="key-confirm-body" class="small">
            {t("keyChange.body")} {t("keyChange.confirmBody", { name: props.name })}
          </p>
          <dl class="kv">
            <dt>{t("keyChange.old")}</dt>
            <dd class="mono">{props.change.old_fingerprint}</dd>
            <dt>{t("keyChange.new")}</dt>
            <dd class="mono">{props.change.new_fingerprint}</dd>
            <dt>{t("keyChange.status")}</dt>
            <dd classList={{ "key-was-verified": props.change.was_verified }}>
              {props.change.was_verified ? t("keyChange.statusVerified") : t("keyChange.statusNotVerified")}
            </dd>
          </dl>
          <p class="small">{t("keyChange.compare", { name: props.name })}</p>
          <div class="row">
            <button class="btn btn-ghost btn-sm" onClick={close}>
              {t("keyChange.cancel")}
            </button>
            <span class="grow" />
            <button class="btn btn-primary btn-sm" onClick={trust}>
              {t("keyChange.trust")}
            </button>
          </div>
        </div>
      </div>
    </Portal>
  );
}
