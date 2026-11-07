// A "⋯" button with a menu of less-used actions (the device page's header).
// Keyboard: Enter, Space or ArrowDown opens it on the first item, ArrowUp on
// the last; arrows move, Home / End jump, Escape or Tab closes, focus goes
// back to the button. An item closes the menu first, then runs, so a dialog
// it opens hands focus back to the button when it closes.

import { createSignal, createUniqueId, For, onCleanup, onMount, Show, type JSX } from "solid-js";
import { Icon, type IconName } from "./Icon";

export type MenuItem = {
  label: string;
  icon?: IconName;
  onSelect: () => void;
  /** Shown in the danger colour, after a divider (e.g. Remove device). */
  danger?: boolean;
  hint?: string;
};

export function MenuButton(props: {
  label: string;
  items: MenuItem[];
  children: JSX.Element;
  /** Screenshot mode: start open. */
  startOpen?: boolean;
}) {
  const id = createUniqueId();
  const [open, setOpen] = createSignal(false);
  let button: HTMLButtonElement | undefined;
  let menu: HTMLDivElement | undefined;
  const items = () => Array.from(menu?.querySelectorAll<HTMLElement>('[role="menuitem"]') ?? []);
  const focusAt = (i: number) => {
    const list = items();
    if (!list.length) return;
    list[(i + list.length) % list.length].focus();
  };
  const show = (at: "first" | "last") => {
    setOpen(true);
    requestAnimationFrame(() => focusAt(at === "first" ? 0 : -1));
  };
  const close = (refocus = true) => {
    setOpen(false);
    if (refocus) button?.focus();
  };
  const pick = (item: MenuItem) => {
    close();
    item.onSelect();
  };
  const onMenuKey = (e: KeyboardEvent) => {
    const list = items();
    const at = list.indexOf(document.activeElement as HTMLElement);
    if (e.key === "ArrowDown") focusAt(at + 1);
    else if (e.key === "ArrowUp") focusAt(at - 1);
    else if (e.key === "Home") focusAt(0);
    else if (e.key === "End") focusAt(-1);
    else if (e.key === "Escape") close();
    else if (e.key === "Tab") {
      close(false);
      return;
    } else return;
    e.preventDefault();
  };
  // A click anywhere else closes it.
  onMount(() => {
    if (props.startOpen) setOpen(true);
    const away = (e: PointerEvent) => {
      if (!open()) return;
      const at = e.target as Node;
      if (!menu?.contains(at) && !button?.contains(at)) close(false);
    };
    document.addEventListener("pointerdown", away);
    onCleanup(() => document.removeEventListener("pointerdown", away));
  });
  return (
    <div class="menu-wrap">
      <button
        ref={button}
        type="button"
        class="btn btn-ghost btn-sm btn-icon"
        aria-label={props.label}
        title={props.label}
        aria-haspopup="menu"
        aria-expanded={open()}
        aria-controls={open() ? id : undefined}
        onClick={() => (open() ? close() : show("first"))}
        onKeyDown={(e) => {
          if (e.key === "ArrowDown" || e.key === "ArrowUp") {
            e.preventDefault();
            show(e.key === "ArrowDown" ? "first" : "last");
          }
        }}
      >
        {props.children}
      </button>
      <Show when={open()}>
        <div ref={menu} id={id} class="menu" role="menu" aria-label={props.label} onKeyDown={onMenuKey}>
          <For each={props.items}>
            {(item) => (
              <>
                <Show when={item.danger}>
                  <div class="menu-sep" role="separator" />
                </Show>
                <button
                  type="button"
                  role="menuitem"
                  tabIndex={-1}
                  class="menu-item"
                  classList={{ "menu-danger": !!item.danger }}
                  title={item.hint}
                  onClick={() => pick(item)}
                >
                  <Show when={item.icon}>{(i) => <Icon name={i()} />}</Show>
                  {item.label}
                </button>
              </>
            )}
          </For>
        </div>
      </Show>
    </div>
  );
}
