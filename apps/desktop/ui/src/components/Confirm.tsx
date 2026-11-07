// A confirm step for destructive actions, asked from anywhere:
//
//   if (await askConfirm({ title: t("chat.clearTitle"), body: t("chat.clearHint"), action: t("chat.clear") })) clearChat(node);
//
// One dialog at a time, drawn by <ConfirmHost/> (mounted once in App).
// Cancel comes first and takes focus (never the default); the action is in
// the danger style. Escape or a click outside cancels.

import { createSignal, Show } from "solid-js";
import { Portal } from "solid-js/web";
import { rememberFocus, trapTab } from "../lib/a11y";
import { t } from "../i18n";
import { Icon } from "./Icon";

type Ask = { title: string; body?: string; action: string; resolve: (yes: boolean) => void };

const [asking, setAsking] = createSignal<Ask | null>(null);

/** Ask before a destructive action; resolves true when confirmed. */
export function askConfirm(o: { title: string; body?: string; action: string }): Promise<boolean> {
  asking()?.resolve(false);
  return new Promise((resolve) => setAsking({ ...o, resolve }));
}

export function ConfirmHost() {
  return (
    <Show when={asking()} keyed>
      {(a) => <ConfirmDialog ask={a} />}
    </Show>
  );
}

function ConfirmDialog(props: { ask: Ask }) {
  let dialog: HTMLDivElement | undefined;
  const restore = rememberFocus();
  const done = (yes: boolean) => {
    setAsking(null);
    props.ask.resolve(yes);
    restore();
  };
  return (
    <Portal>
      <div class="clip-pic-backdrop" onMouseDown={(e) => e.target === e.currentTarget && done(false)}>
        <div
          class="clip-pic-preview key-confirm confirm-dialog"
          ref={(el) => {
            dialog = el;
            queueMicrotask(() => el.querySelector<HTMLElement>("button")?.focus());
          }}
          role="alertdialog"
          aria-modal="true"
          aria-labelledby="confirm-title"
          aria-describedby={props.ask.body ? "confirm-body" : undefined}
          onKeyDown={(e) => {
            if (e.key === "Escape") {
              e.preventDefault();
              done(false);
            } else trapTab(e, dialog);
          }}
        >
          <h2 id="confirm-title">{props.ask.title}</h2>
          <Show when={props.ask.body}>
            <p id="confirm-body" class="small muted">
              {props.ask.body}
            </p>
          </Show>
          <div class="row">
            <button class="btn btn-ghost btn-sm" onClick={() => done(false)}>
              {t("settings.cancel")}
            </button>
            <span class="grow" />
            <button class="btn btn-danger btn-sm" onClick={() => done(true)}>
              <Icon name="remove" size={16} />
              {props.ask.action}
            </button>
          </div>
        </div>
      </div>
    </Portal>
  );
}
