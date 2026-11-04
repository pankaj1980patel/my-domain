// Theme picker: one option per brand-kit theme. Self-contained, so any screen
// can drop it in: <ThemePicker />. Choosing an option switches at once and is
// remembered (lib/theme.ts).
//
// A radio group for the keyboard too: Tab reaches the chosen theme only, and
// the arrow keys (Home / End) move along the themes, choosing as they go.

import { For } from "solid-js";
import { setTheme, theme, THEMES } from "../lib/theme";
import { t, type Key } from "../i18n";

export function ThemePicker() {
  let group: HTMLDivElement | undefined;
  const onKeyDown = (e: KeyboardEvent) => {
    const i = THEMES.findIndex((th) => th.id === theme());
    const n = THEMES.length;
    let next: number;
    switch (e.key) {
      case "ArrowRight":
      case "ArrowDown":
        next = (i + 1) % n;
        break;
      case "ArrowLeft":
      case "ArrowUp":
        next = (i - 1 + n) % n;
        break;
      case "Home":
        next = 0;
        break;
      case "End":
        next = n - 1;
        break;
      default:
        return;
    }
    e.preventDefault();
    setTheme(THEMES[next].id);
    group?.querySelectorAll<HTMLButtonElement>('[role="radio"]')[next]?.focus();
  };
  return (
    <div class="theme-picker" role="radiogroup" aria-label={t("settings.appearance")} ref={group} onKeyDown={onKeyDown}>
      <For each={THEMES}>
        {(th) => (
          <button
            type="button"
            role="radio"
            aria-checked={theme() === th.id}
            tabIndex={theme() === th.id ? 0 : -1}
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
