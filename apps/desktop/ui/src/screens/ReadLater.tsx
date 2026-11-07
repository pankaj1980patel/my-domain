// Read later: links saved to read later, the same on all your devices.
// Paste or type a link (or "Save copied link"), then open it in the browser
// (which marks it read), mark it read or unread, or delete it (with Undo).
// Unread first by default; "All" shows the read ones too. Nothing is
// fetched from the web: a link shows the title it was saved with, else its
// host.

import { Icon } from "../components/Icon";
import { createEffect, createMemo, createSignal, For, Show } from "solid-js";
import {
  deleteLink,
  markRead,
  openLink,
  readLater,
  readLaterMatches,
  restoreLink,
  saveCopiedLink,
  saveLink,
  sortedReadLater,
  type ReadLaterItem,
} from "../lib/readLater";
import { friendlyError, niceName, timeAgo } from "../lib/words";
import { peerIcon, peerName, state } from "../lib/core";
import { DeviceIcon } from "../components/DeviceIcon";
import { Empty, toast } from "../components/ui";
import { foundId, foundTarget, isFound } from "../lib/search";
import { HelpLink } from "../help/HelpLink";
import { t } from "../i18n";

/** One of six tile colours, the same for a site every time. */
function hueOf(host: string): number {
  let h = 0;
  for (const ch of host) h = (h * 31 + ch.charCodeAt(0)) >>> 0;
  return h % 6;
}

export default function ReadLater() {
  const [text, setText] = createSignal("");
  const [title, setTitle] = createSignal("");
  const [query, setQuery] = createSignal("");
  const [all, setAll] = createSignal(false);
  const [busy, setBusy] = createSignal(false);

  const [from, setFrom] = createSignal("");
  /** The devices links were saved on. */
  const sources = createMemo(() => [...new Set(sortedReadLater().map((i) => i.saved_on).filter((x): x is string => !!x))]);
  const list = createMemo(() =>
    sortedReadLater().filter(
      (i) => (all() || !i.read) && readLaterMatches(i, query()) && (!from() || i.saved_on === from()),
    ),
  );
  const total = () => Object.keys(readLater.byId).length;

  // A search result opening a read link: show the read ones too, so its
  // row is there to bring into view.
  createEffect(() => {
    const x = foundTarget();
    if (x?.kind === "read_later" && readLater.byId[x.id]?.read) setAll(true);
  });

  const fail = (e: unknown) => toast(friendlyError(e), "error");
  const saved = (i: ReadLaterItem) => toast(t("readLater.saved", { title: i.title }), "ok");

  const add = async (e: Event) => {
    e.preventDefault();
    if (!text().trim() || busy()) return;
    setBusy(true);
    try {
      saved(await saveLink(text(), title()));
      setText("");
      setTitle("");
    } catch (err) {
      fail(err);
    } finally {
      setBusy(false);
    }
  };
  const addCopied = () => saveCopiedLink().then(saved, fail);
  const remove = (i: ReadLaterItem) =>
    deleteLink(i.id).then(
      () =>
        toast(t("readLater.deleted", { title: i.title }), "info", {
          label: t("readLater.undo"),
          run: () => restoreLink(i).catch(fail),
        }),
      fail,
    );

  return (
    <section class="page page-fit read-later-page">
      <header class="page-head">
        <span class="head-tile" aria-hidden="true">
          <Icon name="readLater" size={26} tone="amber" />
        </span>
        <div class="grow">
          <h1>
            {t("readLater.title")} <HelpLink topic="read-later" />
          </h1>
          <p class="muted">{t("readLater.subtitle")}</p>
        </div>
      </header>

      <form class="card rl-add" onSubmit={add}>
        {/* One way in: type or paste a link, or save the copied one from
            the button inside the field. */}
        <div class="field-with-btn">
          <input
            type="url"
            inputMode="url"
            placeholder={t("readLater.linkPlaceholder")}
            aria-label={t("readLater.linkLabel")}
            value={text()}
            onInput={(e) => setText(e.currentTarget.value)}
          />
          <button
            type="button"
            class="btn btn-ghost btn-sm btn-icon"
            aria-label={t("readLater.saveCopied")}
            title={t("readLater.saveCopied")}
            onClick={addCopied}
          >
            <Icon name="get-clip" size={16} />
          </button>
        </div>
        <input
          type="text"
          placeholder={t("readLater.titlePlaceholder")}
          aria-label={t("readLater.titleLabel")}
          maxLength={300}
          value={title()}
          onInput={(e) => setTitle(e.currentTarget.value)}
        />
        <button class="btn btn-primary btn-sm" type="submit" disabled={!text().trim() || busy()}>
          {t("readLater.save")}
        </button>
      </form>

      <div class="rl-tools">
        <input
          type="search"
          placeholder={t("readLater.search")}
          aria-label={t("readLater.search")}
          value={query()}
          onInput={(e) => setQuery(e.currentTarget.value)}
        />
        {/* Unread or all, then where it was saved (only devices that saved some). */}
        <div class="home-filters" role="group" aria-label={t("readLater.filter")}>
          <button class="filter-chip" aria-pressed={!all()} onClick={() => setAll(false)}>
            {t("readLater.unread")}
          </button>
          <button class="filter-chip" aria-pressed={all()} onClick={() => setAll(true)}>
            {t("readLater.all")}
          </button>
          <Show when={sources().length > 1}>
            <span class="filter-sep" aria-hidden="true" />
            <For each={sources()}>
              {(src) => (
                <button
                  class="filter-chip device-chip"
                  aria-pressed={from() === src}
                  aria-label={t("readLater.savedOn", { name: src })}
                  onClick={() => setFrom(from() === src ? "" : src)}
                >
                  <SourceIcon name={src} />
                  {src}
                </button>
              )}
            </For>
          </Show>
        </div>
      </div>

      <Show
        when={list().length > 0}
        fallback={
          <Empty icon="readLater" title={total() ? t("readLater.noneShown") : t("readLater.emptyTitle")}>
            <p class="muted">
              {query()
                ? t("readLater.noMatch", { query: query() })
                : total()
                  ? t("readLater.allRead")
                  : t("readLater.empty")}
            </p>
          </Empty>
        }
      >
        <ul class="rl-list" aria-label={t("readLater.listLabel")}>
          <For each={list()}>
            {(i) => (
              <li
                class="rl-item"
                classList={{ "rl-read": i.read, "found-target": isFound("read_later", i.id) }}
                id={foundId("read_later", i.id)}
                tabIndex={-1}
              >
                {/* Its site, as a tile (first letter, a colour of its own). */}
                <span class="rl-thumb" data-hue={hueOf(i.host)} aria-hidden="true">
                  {(i.host || "?").replace(/^www\./, "").charAt(0).toUpperCase()}
                </span>
                <button class="rl-open" onClick={() => openLink(i.id).catch(fail)} title={i.url}>
                  <span class="rl-title">{i.title || i.host}</span>
                  <span class="muted small rl-meta">
                    {[i.host, i.saved_on ? t("readLater.savedOn", { name: i.saved_on }) : "", i.saved_at ? timeAgo(i.saved_at) : ""]
                      .filter(Boolean)
                      .join(" · ")}
                  </span>
                </button>
                {/* Icon buttons with tooltips; Delete apart, in the danger
                    tone, with Undo in its toast. */}
                <div class="row rl-actions">
                  <button
                    class="btn btn-ghost btn-sm btn-icon"
                    onClick={() => markRead(i.id, !i.read).catch(fail)}
                    aria-label={t(i.read ? "readLater.markUnreadLabel" : "readLater.markReadLabel", { title: i.title })}
                    title={i.read ? t("readLater.markUnread") : t("readLater.markRead")}
                  >
                    <Icon name={i.read ? "circle-dot" : "check"} size={16} />
                  </button>
                  <button
                    class="btn btn-ghost btn-sm btn-icon btn-danger-ghost rl-delete"
                    onClick={() => remove(i)}
                    aria-label={t("readLater.deleteLabel", { title: i.title })}
                    title={t("readLater.delete")}
                  >
                    <Icon name="remove" size={16} />
                  </button>
                </div>
              </li>
            )}
          </For>
        </ul>
      </Show>
    </section>
  );
}

/** The device a link was saved on, as its icon (when it's one of yours). */
function SourceIcon(props: { name: string }) {
  const node = () => state.peers.find((p) => niceName(peerName(p.node_id)) === niceName(props.name))?.node_id;
  return (
    <Show when={node()} fallback={<Icon name="devices" size={14} />}>
      {(n) => <DeviceIcon icon={peerIcon(n())} size={14} />}
    </Show>
  );
}
