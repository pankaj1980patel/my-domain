// One setting: its name and a short description on the left, its control on
// the right (Settings uses this pattern for every row).

import { createUniqueId, Show, type JSX } from "solid-js";
import { Icon, type IconName } from "./Icon";

/** A switch. The whole row toggles it; the switch is named by the label
 *  and described by the hint. */
export function SwitchRow(props: {
  icon?: IconName;
  label: string;
  hint?: JSX.Element;
  checked: boolean;
  disabled?: boolean;
  onChange: (on: boolean) => void;
}) {
  const hint = createUniqueId();
  return (
    <label class="set-row switch-row">
      <Show when={props.icon}>{(i) => <Icon name={i()} size={20} class="set-icon" />}</Show>
      <span class="set-text">
        <span class="set-label">{props.label}</span>
        <Show when={props.hint}>
          <span class="set-hint" id={hint}>
            {props.hint}
          </span>
        </Show>
      </span>
      <input
        type="checkbox"
        role="switch"
        aria-label={props.label}
        aria-describedby={props.hint ? hint : undefined}
        checked={props.checked}
        disabled={props.disabled}
        onChange={(e) => props.onChange(e.currentTarget.checked)}
      />
    </label>
  );
}

/** Any other control (a select, a button): `children` on the right. */
export function SettingRow(props: { icon?: IconName; label: string; hint?: JSX.Element; children: JSX.Element }) {
  return (
    <div class="set-row">
      <Show when={props.icon}>{(i) => <Icon name={i()} size={20} class="set-icon" />}</Show>
      <span class="set-text">
        <span class="set-label">{props.label}</span>
        <Show when={props.hint}>
          <span class="set-hint">{props.hint}</span>
        </Show>
      </span>
      <span class="set-control">{props.children}</span>
    </div>
  );
}
