// A device's look here: its avatar (the icon you chose, else the one for
// what it is: a Mac laptop, an Android phone..., ringed in the colour you
// chose) and the small form that sets the
// nickname, icon and colour. Kept here and shared with your other devices
// while the Settings switch is on (`device_labels.rs`);
// names everywhere come from `peerName`, which prefers the nickname.

import { createSignal, For, onMount, Show } from "solid-js";
import { rememberFocus } from "../lib/a11y";
import { autoIcon, peerIcon, peerLabel, setDeviceLabel, state } from "../lib/core";
import { friendlyError, niceName } from "../lib/words";
import { DEVICE_ICONS, DeviceIcon } from "./DeviceIcon";
import { toast } from "./ui";
import { t, type Key } from "../i18n";

/** Icons a device can have (same set as `device_labels::ICONS`). */
export const LABEL_ICONS = DEVICE_ICONS;
export { DeviceIcon };
/** Accent colours (same set as `device_labels::COLORS`; app.css maps each
 *  to a theme colour). */
export const LABEL_COLORS = ["cyan", "blue", "sky", "green", "gold", "rose", "grey"] as const;

/** A device's avatar: the icon you chose, else the one for what it is
 *  (`peerIcon`), in its colour. */
export function DeviceAvatar(props: { node: string; large?: boolean; small?: boolean }) {
  const label = () => peerLabel(props.node);
  return (
    <div
      class="device-avatar"
      classList={{ lg: !!props.large, sm: !!props.small }}
      data-color={label()?.color ?? undefined}
      aria-hidden="true"
    >
      <DeviceIcon icon={peerIcon(props.node)} size={props.large ? 28 : props.small ? 16 : 24} />
    </div>
  );
}

/** The nickname / icon / colour form, for a device's page. */
export function DeviceLabelForm(props: { node: string; onClose: () => void }) {
  const current = peerLabel(props.node);
  const own = () => niceName(state.peers.find((p) => p.node_id === props.node)?.name ?? "");
  const [nickname, setNickname] = createSignal(current?.nickname ?? "");
  const [icon, setIcon] = createSignal<string | null>(current?.icon ?? null);
  const [color, setColor] = createSignal<string | null>(current?.color ?? null);
  const [busy, setBusy] = createSignal(false);
  // Opened from the page's Edit button: the nickname takes focus, and focus
  // goes back to Edit when the form closes (saved, cancelled or Esc).
  const restore = rememberFocus();
  let nicknameInput: HTMLInputElement | undefined;
  onMount(() => requestAnimationFrame(() => nicknameInput?.focus()));
  const close = () => {
    props.onClose();
    restore();
  };

  const save = async (n: string, i: string | null, c: string | null) => {
    setBusy(true);
    try {
      await setDeviceLabel(props.node, n, i, c);
      toast(t("label.saved"), "ok");
      close();
    } catch (e) {
      toast(friendlyError(e), "error");
    } finally {
      setBusy(false);
    }
  };

  return (
    <form
      class="label-form"
      aria-labelledby="label-form-title"
      onSubmit={(e) => {
        e.preventDefault();
        void save(nickname(), icon(), color());
      }}
      onKeyDown={(e) => {
        if (e.key === "Escape") {
          e.preventDefault();
          close();
        }
      }}
    >
      <h2 id="label-form-title">{t("label.title")}</h2>
      <p class="muted small">{t("label.hint")}</p>
      <label class="field">
        <span>{t("label.nickname")}</span>
        <input
          type="text"
          maxLength={40}
          value={nickname()}
          placeholder={t("label.nicknamePlaceholder", { name: own() })}
          onInput={(e) => setNickname(e.currentTarget.value)}
          ref={nicknameInput}
        />
      </label>
      <fieldset class="label-choices">
        <legend>{t("label.icon")}</legend>
        <button
          type="button"
          class="label-choice"
          classList={{ on: icon() === null }}
          aria-pressed={icon() === null}
          aria-label={t("label.auto")}
          title={t("label.auto")}
          onClick={() => setIcon(null)}
        >
          <DeviceIcon icon={autoIcon(props.node)} />
          <span class="label-auto">{t("label.autoShort")}</span>
        </button>
        <For each={LABEL_ICONS}>
          {(i) => (
            <button
              type="button"
              class="label-choice"
              classList={{ on: icon() === i }}
              aria-pressed={icon() === i}
              aria-label={t(`label.icon.${i}` as Key)}
              title={t(`label.icon.${i}` as Key)}
              onClick={() => setIcon(i)}
            >
              <DeviceIcon icon={i} />
            </button>
          )}
        </For>
      </fieldset>
      <fieldset class="label-choices">
        <legend>{t("label.color")}</legend>
        <button
          type="button"
          class="label-swatch label-swatch-none"
          classList={{ on: color() === null }}
          aria-pressed={color() === null}
          aria-label={t("label.none")}
          title={t("label.none")}
          onClick={() => setColor(null)}
        />
        <For each={LABEL_COLORS}>
          {(c) => (
            <button
              type="button"
              class="label-swatch"
              data-color={c}
              classList={{ on: color() === c }}
              aria-pressed={color() === c}
              aria-label={t(`label.color.${c}` as Key)}
              title={t(`label.color.${c}` as Key)}
              onClick={() => setColor(c)}
            />
          )}
        </For>
      </fieldset>
      <div class="row label-actions">
        <button type="submit" class="btn btn-primary btn-sm" disabled={busy()}>
          {t("label.save")}
        </button>
        <button type="button" class="btn btn-ghost btn-sm" disabled={busy()} onClick={close}>

          {t("label.cancel")}
        </button>
        <Show when={current}>
          <button type="button" class="btn btn-ghost btn-sm" disabled={busy()} onClick={() => void save("", null, null)}>
            {t("label.reset")}
          </button>
        </Show>
      </div>
    </form>
  );
}
