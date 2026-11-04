// A device's look here: its avatar (the icon you chose, else the initial of
// its name, ringed in the colour you chose) and the small form that sets the
// nickname, icon and colour. Kept on this computer only (`device_labels.rs`);
// names everywhere come from `peerName`, which prefers the nickname.

import { createSignal, For, Show } from "solid-js";
import { peerLabel, setDeviceLabel, state } from "../lib/core";
import { friendlyError, initial, niceName } from "../lib/words";
import { toast } from "./ui";
import { t, type Key } from "../i18n";

/** Icons a device can have (same set as `device_labels::ICONS`). */
export const LABEL_ICONS = ["phone", "tablet", "laptop", "desktop", "tv", "watch"] as const;
/** Accent colours (same set as `device_labels::COLORS`; app.css maps each
 *  to a theme colour). */
export const LABEL_COLORS = ["cyan", "blue", "sky", "green", "gold", "rose", "grey"] as const;

/** One device icon, drawn in the current text colour. */
export function DeviceIcon(props: { icon: string; size?: number }) {
  const size = () => props.size ?? 24;
  return (
    <svg
      class="device-icon"
      width={size()}
      height={size()}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      stroke-width="1.8"
      stroke-linecap="round"
      stroke-linejoin="round"
      aria-hidden="true"
    >
      {(() => {
        switch (props.icon) {
          case "phone":
            return (
              <>
                <rect x="7" y="2" width="10" height="20" rx="2" />
                <path d="M11 18h2" />
              </>
            );
          case "tablet":
            return (
              <>
                <rect x="4" y="2" width="16" height="20" rx="2" />
                <path d="M11 18h2" />
              </>
            );
          case "laptop":
            return (
              <>
                <rect x="4" y="5" width="16" height="11" rx="1.5" />
                <path d="M2 19h20" />
              </>
            );
          case "desktop":
            return (
              <>
                <rect x="3" y="4" width="18" height="12" rx="1.5" />
                <path d="M8 20h8M12 16v4" />
              </>
            );
          case "tv":
            return (
              <>
                <rect x="2" y="7" width="20" height="13" rx="2" />
                <path d="M8 3l4 4 4-4" />
              </>
            );
          case "watch":
            return (
              <>
                <rect x="6" y="6" width="12" height="12" rx="3" />
                <path d="M9 6l1-4h4l1 4M9 18l1 4h4l1-4" />
              </>
            );
          default:
            return null;
        }
      })()}
    </svg>
  );
}

/** A device's avatar: its icon (else its initial), in its colour. */
export function DeviceAvatar(props: { node: string; large?: boolean }) {
  const label = () => peerLabel(props.node);
  const own = () => state.peers.find((p) => p.node_id === props.node)?.name ?? "";
  return (
    <div class="device-avatar" classList={{ lg: !!props.large }} data-color={label()?.color ?? undefined} aria-hidden="true">
      <Show when={label()?.icon} fallback={<span>{initial(label()?.nickname || own())}</span>}>
        {(icon) => <DeviceIcon icon={icon()} size={props.large ? 28 : 24} />}
      </Show>
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

  const save = async (n: string, i: string | null, c: string | null) => {
    setBusy(true);
    try {
      await setDeviceLabel(props.node, n, i, c);
      toast(t("label.saved"), "ok");
      props.onClose();
    } catch (e) {
      toast(friendlyError(e), "error");
    } finally {
      setBusy(false);
    }
  };

  return (
    <form
      class="label-form"
      onSubmit={(e) => {
        e.preventDefault();
        void save(nickname(), icon(), color());
      }}
      onKeyDown={(e) => e.key === "Escape" && props.onClose()}
    >
      <h2>{t("label.title")}</h2>
      <p class="muted small">{t("label.hint", { name: own() })}</p>
      <label class="field">
        <span>{t("label.nickname")}</span>
        <input
          type="text"
          maxLength={40}
          value={nickname()}
          placeholder={t("label.nicknamePlaceholder", { name: own() })}
          onInput={(e) => setNickname(e.currentTarget.value)}
          autofocus
        />
      </label>
      <fieldset class="label-choices">
        <legend>{t("label.icon")}</legend>
        <button
          type="button"
          class="label-choice"
          classList={{ on: icon() === null }}
          aria-pressed={icon() === null}
          title={t("label.none")}
          onClick={() => setIcon(null)}
        >
          <span class="label-initial">{initial(nickname() || own())}</span>
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
        <button type="button" class="btn btn-ghost btn-sm" disabled={busy()} onClick={props.onClose}>
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
