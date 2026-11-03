// Theme picker: one option per brand-kit theme. Self-contained, so any screen
// can drop it in: <ThemePicker />. Choosing an option switches at once and is
// remembered (lib/theme.ts).

import { For } from "solid-js";
import { setTheme, theme, THEMES } from "../lib/theme";
import { t, type Key } from "../i18n";

export function ThemePicker() {
  return (
    <div class="theme-picker" role="radiogroup" aria-label={t("settings.appearance")}>
      <For each={THEMES}>
        {(th) => (
          <button
            type="button"
            role="radio"
            aria-checked={theme() === th.id}
            class={`theme-option ${theme() === th.id ? "selected" : ""}`}
            onClick={() => setTheme(th.id)}
          >
            <span class="theme-swatches" aria-hidden="true">
              <For each={th.swatch}>{(c) => <span style={{ background: c }} />}</For>
            </span>
            <span class="theme-name">{t(`theme.${th.id}` as Key)}</span>
            <span class="theme-desc">{t(`theme.${th.id}.desc` as Key)}</span>
          </button>
        )}
      </For>
    </div>
  );
}
