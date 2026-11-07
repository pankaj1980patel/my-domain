// A row of tabs (WAI-ARIA tabs pattern): one tab stop, Left / Right move
// and select (automatic activation), Home / End jump. The panel is the
// caller's: give it `panelProps(id)` so it is labelled by its tab.

import { For } from "solid-js";

import { Show } from "solid-js";
import { Icon, type IconName } from "./Icon";

export type TabDef = { id: string; label: string; icon?: IconName; count?: number };

export function Tabs(props: {
  label: string;
  /** Prefix for the tab and panel element ids. */
  idBase: string;
  tabs: TabDef[];
  selected: string;
  onSelect: (id: string) => void;
}) {
  let list: HTMLDivElement | undefined;
  const move = (e: KeyboardEvent) => {
    const ids = props.tabs.map((x) => x.id);
    const at = ids.indexOf(props.selected);
    let next = -1;
    if (e.key === "ArrowRight") next = (at + 1) % ids.length;
    else if (e.key === "ArrowLeft") next = (at - 1 + ids.length) % ids.length;
    else if (e.key === "Home") next = 0;
    else if (e.key === "End") next = ids.length - 1;
    if (next < 0) return;
    e.preventDefault();
    props.onSelect(ids[next]);
    queueMicrotask(() => list?.querySelector<HTMLElement>(`#${props.idBase}-tab-${ids[next]}`)?.focus());
  };
  return (
    <div ref={list} class="tabs" role="tablist" aria-label={props.label} onKeyDown={move}>
      <For each={props.tabs}>
        {(tab) => {
          const on = () => tab.id === props.selected;
          return (
            <button
              type="button"
              role="tab"
              id={`${props.idBase}-tab-${tab.id}`}
              class="tab"
              aria-selected={on()}
              aria-controls={`${props.idBase}-panel`}
              tabIndex={on() ? 0 : -1}
              title={tab.label}
              onClick={() => props.onSelect(tab.id)}
            >
              <Show when={tab.icon}>{(i) => <Icon name={i()} />}</Show>
              <span class="tab-label">{tab.label}</span>
              {tab.count ? (
                <span class="tab-count">
                  {tab.count}
                </span>
              ) : null}
            </button>
          );
        }}
      </For>
    </div>
  );
}

/** The attributes the selected tab's panel takes. */
export function panelProps(idBase: string, selected: string) {
  return {
    id: `${idBase}-panel`,
    role: "tabpanel",
    "aria-labelledby": `${idBase}-tab-${selected}`,
    tabIndex: 0,
  } as const;
}
