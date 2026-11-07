// Search: everything this computer keeps, in one place (lib/search.ts). A
// search box, filters (what, which device, when), and the results by
// group, with more per group than the palette shows. ↑/↓ move through every
// group from the box, Enter opens one where it lives; "Show all" on a group
// narrows to it. Only data on this computer is searched.

import { Icon } from "../components/Icon";
import { MenuButton } from "../components/Menu";
import { createEffect, createMemo, createSignal, For, on, onCleanup, onMount, Show } from "solid-js";
import { peerName, state } from "../lib/core";
import { FOUND_KINDS, KIND_LABEL, queryWords, searchAll, type Found, type FoundGroup, type FoundKind } from "../lib/search";
import { FoundText } from "../components/Found";
import { t, tn } from "../i18n";

/** Results per group with every kind shown, and with one kind. */
const PER_GROUP = 20;
const ONE_KIND = 200;
const SEARCH_DELAY = 150;
const DAY_MS = 24 * 3600 * 1000;

type When = "any" | "today" | "week" | "month" | "custom";

// Kept while the app runs, so coming back from a result finds them as left.
const [query, setQuery] = createSignal("");
const [kind, setKind] = createSignal<FoundKind | "">("");
/** The kinds with a chip of their own. */
const MAIN_KINDS: FoundKind[] = ["devices", "transfers", "messages", "notes", "automations", "settings"];
const [node, setNode] = createSignal("");
const [when, setWhen] = createSignal<When>("any");
const [from, setFrom] = createSignal("");
const [to, setTo] = createSignal("");

/** A date input's value ("2026-11-05") as local midnight, in ms. */
function dayStart(v: string): number | null {
  const m = /^(\d{4})-(\d{2})-(\d{2})$/.exec(v);
  return m ? new Date(Number(m[1]), Number(m[2]) - 1, Number(m[3])).getTime() : null;
}

/** The chosen span, in ms since 1970. */
function span(): { since: number | null; until: number | null } {
  const now = Date.now();
  switch (when()) {
    case "today": {
      const d = new Date();
      return { since: new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime(), until: null };
    }
    case "week":
      return { since: now - 7 * DAY_MS, until: null };
    case "month":
      return { since: now - 30 * DAY_MS, until: null };
    case "custom": {
      const end = dayStart(to());
      return { since: dayStart(from()), until: end === null ? null : end + DAY_MS - 1 };
    }
    default:
      return { since: null, until: null };
  }
}

type Row = { type: "hit"; hit: Found } | { type: "more"; kind: FoundKind; total: number };

export default function Search(props: { query?: string; onOpen: (f: Found) => void }) {
  const [groups, setGroups] = createSignal<FoundGroup[]>([]);
  const [busy, setBusy] = createSignal(false);
  const [active, setActive] = createSignal(-1);
  let input: HTMLInputElement | undefined;
  let list: HTMLDivElement | undefined;

  // Opened with a query (the palette's "See all results"): that one.
  createEffect(
    on(
      () => props.query,
      (q) => {
        if (q) setQuery(q);
      },
    ),
  );
  onMount(() => {
    input?.focus();
    input?.select();
  });

  const words = createMemo(() => queryWords(query()));
  const filtered = () => !!node() || when() !== "any";

  let asked = 0;
  let timer: number | undefined;
  createEffect(
    on([query, kind, node, when, from, to], ([q, k, n]) => {
      const mine = ++asked;
      clearTimeout(timer);
      if (!queryWords(q).length && !filtered()) {
        setGroups([]);
        setBusy(false);
        return;
      }
      setBusy(true);
      timer = window.setTimeout(() => {
        const s = span();
        searchAll(q, {
          limit: k ? ONE_KIND : PER_GROUP,
          kinds: k ? [k] : undefined,
          node: n || null,
          since: s.since,
          until: s.until,
        }).then(
          (g) => {
            if (mine !== asked) return;
            setGroups(g);
            setBusy(false);
          },
          () => mine === asked && setBusy(false),
        );
      }, SEARCH_DELAY);
    }),
  );
  onCleanup(() => clearTimeout(timer));

  /** Every row in order, a group's "Show all" after its hits. */
  const rows = createMemo<Row[]>(() =>
    groups().flatMap((g) => [
      ...g.hits.map((hit): Row => ({ type: "hit", hit })),
      ...(!kind() && g.total > g.hits.length ? [{ type: "more", kind: g.kind, total: g.total } as Row] : []),
    ]),
  );
  const groupStart = (gi: number) =>
    groups()
      .slice(0, gi)
      .reduce((n, g) => n + g.hits.length + (!kind() && g.total > g.hits.length ? 1 : 0), 0);
  const total = () => groups().reduce((n, g) => n + g.total, 0);

  // New results: nothing chosen until an arrow is pressed.
  createEffect(on(groups, () => setActive(-1), { defer: true }));
  createEffect(
    on(active, (i) => {
      if (i >= 0) list?.querySelector(`#search-opt-${i}`)?.scrollIntoView({ block: "nearest" });
    }),
  );

  const optionId = (i: number) => `search-opt-${i}`;
  const runRow = (r: Row | undefined) => {
    if (!r) return;
    if (r.type === "hit") props.onOpen(r.hit);
    else {
      setKind(r.kind);
      input?.focus();
    }
  };
  const move = (by: number) => {
    const n = rows().length;
    if (!n) return;
    setActive((i) => (i < 0 ? (by > 0 ? 0 : n - 1) : (i + by + n) % n));
  };
  const onKeyDown = (e: KeyboardEvent) => {
    switch (e.key) {
      case "ArrowDown":
        e.preventDefault();
        move(1);
        break;
      case "ArrowUp":
        e.preventDefault();
        move(-1);
        break;
      case "Enter":
        if (active() >= 0) {
          e.preventDefault();
          runRow(rows()[active()]);
        }
        break;
      case "Escape":
        if (active() >= 0) {
          e.preventDefault();
          setActive(-1);
        }
        break;
    }
  };

  const reset = () => {
    setKind("");
    setNode("");
    setWhen("any");
    setFrom("");
    setTo("");
  };

  return (
    <section class="page page-fit search-page">
      <header class="page-head">
        <span class="head-tile" aria-hidden="true">
          <Icon name="search" size={26} tone="blue" />
        </span>
        <div class="grow">
          <h1>{t("search.title")}</h1>
          <p class="muted">{t("search.subtitle")}</p>
        </div>
      </header>
      <div class="search-box">
        <input
          ref={input}
          type="search"
          class="search-input"
          role="combobox"
          aria-expanded={rows().length > 0}
          aria-controls="search-results"
          aria-autocomplete="list"
          aria-activedescendant={active() >= 0 ? optionId(active()) : undefined}
          aria-label={t("search.inputLabel")}
          placeholder={t("search.placeholder")}
          autocomplete="off"
          spellcheck={false}
          value={query()}
          onInput={(e) => setQuery(e.currentTarget.value)}
          onKeyDown={onKeyDown}
        />
      </div>
      <div class="search-filters" role="group" aria-label={t("search.filters")}>
        {/* What kind: the main ones as chips, the rest under More. */}
        <div class="home-filters search-chips" role="group" aria-label={t("search.filterKind")}>
          <button class="filter-chip" aria-pressed={kind() === ""} onClick={() => setKind("")}>
            {t("search.anyKind")}
          </button>
          <For each={MAIN_KINDS}>
            {(k) => (
              <button class="filter-chip" aria-pressed={kind() === k} onClick={() => setKind(k)}>
                {t(KIND_LABEL[k])}
              </button>
            )}
          </For>
          <MenuButton
            label={t("search.moreKinds")}
            labelled
            triggerClass="filter-chip"
            items={FOUND_KINDS.filter((k) => !MAIN_KINDS.includes(k)).map((k) => ({ label: t(KIND_LABEL[k]), onSelect: () => setKind(k) }))}
          >
            {kind() && !MAIN_KINDS.includes(kind() as FoundKind) ? t(KIND_LABEL[kind() as FoundKind]) : t("search.moreKinds")}
            <Icon name="chevron-down" size={14} />
          </MenuButton>
        </div>
        {/* Device and when: one group at the right, each with its icon. */}
        <div class="search-selects">
        <label class="select-with-icon">
          <Icon name="devices" size={16} />
          <select aria-label={t("search.filterDevice")} value={node()} onChange={(e) => setNode(e.currentTarget.value)}>
            <option value="">{t("search.anyDevice")}</option>
            <For each={state.peers}>{(p) => <option value={p.node_id}>{peerName(p.node_id)}</option>}</For>
          </select>
        </label>
        <label class="select-with-icon">
          <Icon name="calendar" size={16} />
          <select aria-label={t("search.filterWhen")} value={when()} onChange={(e) => setWhen(e.currentTarget.value as When)}>
            <option value="any">{t("search.anyTime")}</option>
            <option value="today">{t("search.today")}</option>
            <option value="week">{t("search.week")}</option>
            <option value="month">{t("search.month")}</option>
            <option value="custom">{t("search.custom")}</option>
          </select>
        </label>
        </div>
        <Show when={when() === "custom"}>
          <label class="search-filter">
            <span class="small muted">{t("search.dateFrom")}</span>
            <input type="date" value={from()} max={to() || undefined} onInput={(e) => setFrom(e.currentTarget.value)} />
          </label>
          <label class="search-filter">
            <span class="small muted">{t("search.dateTo")}</span>
            <input type="date" value={to()} min={from() || undefined} onInput={(e) => setTo(e.currentTarget.value)} />
          </label>
        </Show>
        <Show when={kind() || filtered()}>
          <button class="btn btn-ghost btn-sm search-reset" onClick={reset}>
            {t("search.clearFilters")}
          </button>
        </Show>
      </div>
      {/* Said after each search (the list itself isn't live). */}
      <p class="sr-only" role="status">
        {busy() ? "" : words().length || filtered() ? (total() ? tn("a11y.foundCount", total()) : t("search.none")) : ""}
      </p>
      <Show
        when={words().length || filtered()}
        fallback={<p class="muted search-hint">{t("search.start")}</p>}
      >
        <Show when={groups().length || busy()} fallback={<p class="muted search-hint">{t("search.none")}</p>}>
          <div class="search-results" id="search-results" role="listbox" aria-label={t("search.results")} ref={list}>
            <For each={groups()}>
              {(g, gi) => (
                <div class="search-group" role="group" aria-labelledby={`search-group-${g.kind}`}>
                  <h2 class="search-group-head" id={`search-group-${g.kind}`}>
                    {t(KIND_LABEL[g.kind])}
                    <span class="search-group-count small muted">{g.total}</span>
                  </h2>
                  <For each={g.hits}>
                    {(h, hi) => {
                      const i = () => groupStart(gi()) + hi();
                      return (
                        <div
                          id={optionId(i())}
                          class="search-option"
                          classList={{ "search-active": i() === active() }}
                          role="option"
                          aria-selected={i() === active()}
                          onMouseDown={(e) => e.preventDefault()}
                          onClick={() => {
                            setActive(i());
                            runRow({ type: "hit", hit: h });
                          }}
                        >
                          <FoundText f={h} words={words()} />
                        </div>
                      );
                    }}
                  </For>
                  <Show when={!kind() && g.total > g.hits.length}>
                    {(() => {
                      const i = () => groupStart(gi()) + g.hits.length;
                      return (
                        <div
                          id={optionId(i())}
                          class="search-option search-more small"
                          classList={{ "search-active": i() === active() }}
                          role="option"
                          aria-selected={i() === active()}
                          onMouseDown={(e) => e.preventDefault()}
                          onClick={() => runRow({ type: "more", kind: g.kind, total: g.total })}
                        >
                          {t("search.showAll", { count: g.total, kind: t(KIND_LABEL[g.kind]) })}
                        </div>
                      );
                    })()}
                  </Show>
                </div>
              )}
            </For>
          </div>
        </Show>
      </Show>
    </section>
  );
}
