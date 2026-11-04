// Lost mode for a phone: your message (and a number to call) full screen
// over its lock screen, one loud ring, and where it is if location sharing is
// on there. The phone stays locked. Shown when the phone lists `lost`; its
// status says `lost: true` while it's on.

import { createSignal, Show } from "solid-js";
import { peerName, runAction, state } from "../../lib/core";
import { friendlyError } from "../../lib/words";
import { toast } from "../../components/ui";
import { t } from "../../i18n";
import { focusSoon } from "../../lib/a11y";

/** The phone keeps this much of the message. */
const MAX_MESSAGE = 500;

export default function LostModeTile(props: { node: string }) {
  const st = () => state.status[props.node];
  const name = () => peerName(props.node);
  const [open, setOpen] = createSignal(false);
  const [message, setMessage] = createSignal("");
  const [contact, setContact] = createSignal("");
  const [busy, setBusy] = createSignal(false);
  // Keyboard: opening the form puts focus in the message; closing it puts
  // focus back on what's there instead (Lost mode…, or Stop once it's on).
  let openButton: HTMLButtonElement | undefined;
  let messageBox: HTMLTextAreaElement | undefined;
  let stopButton: HTMLButtonElement | undefined;
  const openForm = () => {
    setOpen(true);
    focusSoon(() => messageBox);
  };
  const closeForm = () => {
    setOpen(false);
    focusSoon(() => [stopButton, openButton].find((el) => el?.isConnected));
  };

  const start = async (e: Event) => {
    e.preventDefault();
    setBusy(true);
    try {
      const via = await runAction(props.node, "lost", { on: true, message: message().trim(), contact: contact().trim() });
      toast(via === "queued" ? t("lost.queued", { name: name() }) : t("lost.started", { name: name() }), "ok");
      closeForm();
    } catch (err) {
      toast(friendlyError(err), "error");
    } finally {
      setBusy(false);
    }
  };
  const stop = () =>
    runAction(props.node, "lost", { on: false })
      .then(() => toast(t("lost.stopped", { name: name() })))
      .catch((e) => toast(friendlyError(e), "error"));

  return (
    <Show when={st()?.actions.includes("lost")}>
      <article class="tile">
        <h2>{t("lost.title")}</h2>
        <Show
          when={st()?.lost}
          fallback={
            <Show
              when={open()}
              fallback={
                <>
                  <p class="muted small">{t("lost.hint", { name: name() })}</p>
                  <div class="row">
                    <button class="btn btn-soft" ref={openButton} onClick={openForm}>
                      {t("lost.open")}
                    </button>
                  </div>
                </>
              }
            >
              <form
                class="stack"
                aria-label={t("lost.title")}
                onSubmit={start}
                onKeyDown={(e) => {
                  if (e.key === "Escape") {
                    e.preventDefault();
                    closeForm();
                  }
                }}
              >
                <label class="field">
                  <span>{t("lost.message")}</span>
                  <textarea
                    ref={messageBox}
                    class="lost-message"
                    rows="3"
                    maxLength={MAX_MESSAGE}
                    placeholder={t("lost.messagePlaceholder")}
                    value={message()}
                    onInput={(e) => setMessage(e.currentTarget.value)}
                  />
                </label>
                <label class="field">
                  <span>{t("lost.contact")}</span>
                  <input
                    type="tel"
                    maxLength={20}
                    placeholder={t("lost.contactPlaceholder")}
                    value={contact()}
                    onInput={(e) => setContact(e.currentTarget.value)}
                  />
                </label>
                <p class="muted small">{t("lost.startHint")}</p>
                <div class="row">
                  <button class="btn btn-primary" type="submit" disabled={busy()}>
                    {t("lost.start")}
                  </button>
                  <button class="btn btn-ghost" type="button" onClick={closeForm}>
                    {t("lost.cancel")}
                  </button>
                </div>
              </form>
            </Show>
          }
        >
          <div class="lost-banner row" role="status">
            <span class="grow">
              <strong>{t("lost.on")}</strong>
              <span class="muted small lost-banner-hint">{t("lost.onHint", { name: name() })}</span>
            </span>
            <button class="btn btn-soft btn-sm" ref={stopButton} onClick={stop}>

              {t("lost.stop")}
            </button>
          </div>
        </Show>
      </article>
    </Show>
  );
}
