// Clipboard history for one device: the text sent to it and received from it
// as clipboard, kept on this computer (the last 200, clip_history.rs),
// searchable. "Copy" puts a clip on this computer's clipboard; "Send again"
// puts it on the device's. Turned off in Settings, nothing is kept.
// Copied pictures are kept too (the newest 50, 100 MB at most): a row with a
// thumbnail, its size, and Copy / Send again / Save as… / Open (larger, here).

import { createEffect, createSignal, For, on, Show } from "solid-js";
import { Portal } from "solid-js/web";
import {
  clearClipHistoryOf,
  clipHistory,
  clipPictureUrl,
  clipTick,
  copyClipPicture,
  copyText,
  peerName,
  saveClipPicture,
  sendClipAgain,
  sendClipPictureAgain,
  type ClipPage,
  type ClipPicture,
  type ClipRecord,
} from "../../lib/core";
import { foundId, foundTarget, isFound } from "../../lib/search";
import { friendlyError, humanSize, clockTime, shortDate } from "../../lib/words";
import { rememberFocus, trapTab } from "../../lib/a11y";
import { toast } from "../../components/ui";
import { t } from "../../i18n";
import { askConfirm } from "../../components/Confirm";
import { MenuButton } from "../../components/Menu";
import { Icon } from "../../components/Icon";

/** Rows shown at first, and added by "Show more". */
const PAGE = 50;
/** Every kept clip (clip_history.rs keeps 200), to show one a search opened. */
const ALL = 200;

/** The time, with the date when it wasn't today. */
function whenOf(ts: number, now = new Date()): string {
  const d = new Date(ts);
  const time = clockTime(d.getTime());
  if (d.toDateString() === now.toDateString()) return time;
  return `${shortDate(d.getTime())} ${time}`;
}

/** A picture's dimensions and size: "800 × 600 · 1.2 MB". */
function details(pic: ClipPicture): string {
  return t("clipHistory.pictureDetails", { w: pic.width, h: pic.height, size: humanSize(pic.bytes ?? 0) });
}

export default function ClipHistoryTile(props: { node: string }) {
  const name = () => peerName(props.node);
  const [query, setQuery] = createSignal("");
  const [limit, setLimit] = createSignal(PAGE);
  const [page, setPage] = createSignal<ClipPage>({ entries: [], total: 0, kept: 0, pictures: [] });
  /** The picture shown larger ("Open"). */
  const [preview, setPreview] = createSignal<{ pic: ClipPicture; e: ClipRecord; url: string } | null>(null);
  let asked = 0;

  // A new search starts from the top again.
  createEffect(on([query, () => props.node], () => setLimit(PAGE), { defer: true }));
  // A clip opened from a search: every clip, so it's in the list.
  createEffect(
    on(foundTarget, (x) => {
      if (x?.kind !== "clipboard" || x.node !== props.node) return;
      setQuery("");
      queueMicrotask(() => setLimit(ALL));
    }),
  );
  createEffect(
    on([() => props.node, query, limit, clipTick], ([node, q, l]) => {
      const mine = ++asked;
      clipHistory(node, q, l)
        .then((p) => {
          if (mine === asked) setPage(p);
        })
        .catch(() => {});
    }),
  );

  const way = (e: ClipRecord) =>
    e.direction === "in" ? t("clipHistory.from", { name: name() }) : t("clipHistory.to", { name: name() });
  const pictureOf = (id: number) => page().pictures.find((p) => p.id === id);

  const copy = (text: string) =>
    copyText(text)
      .then(() => toast(t("clipHistory.copied"), "ok"))
      .catch((e) => toast(friendlyError(e), "error"));
  const sendAgain = (text: string) =>
    sendClipAgain(props.node, text)
      .then((how) =>
        toast(how === "queued" ? t("clipHistory.queued", { name: name() }) : t("clipHistory.sent", { name: name() }), "ok"),
      )
      .catch((e) => toast(friendlyError(e), "error"));
  const copyPicture = (id: number) =>
    copyClipPicture(id)
      .then(() => toast(t("clipHistory.pictureCopied"), "ok"))
      .catch((e) => toast(friendlyError(e), "error"));
  const sendPicture = (id: number) =>
    sendClipPictureAgain(props.node, id)
      .then(() => toast(t("clipHistory.pictureSending", { name: name() }), "ok"))
      .catch((e) => toast(friendlyError(e), "error"));
  const savePicture = (id: number) =>
    saveClipPicture(id)
      .then((path) => {
        if (path) toast(t("clipHistory.pictureSaved"), "ok");
      })
      .catch((e) => toast(friendlyError(e), "error"));
  const openPicture = (pic: ClipPicture, e: ClipRecord) =>
    clipPictureUrl(pic.id)
      .then((url) => setPreview({ pic, e, url }))
      .catch((err) => toast(friendlyError(err), "error"));
  const clear = () =>
    clearClipHistoryOf(props.node)
      .then(() => toast(t("clipHistory.cleared"), "ok"))
      .catch((e) => toast(friendlyError(e), "error"));

  return (
    <article class="tile tile-wide">
      <div class="tile-head">
        <h2>{t("clipHistory.title")}</h2>
        {/* Clear is destructive: in the card's "⋯", asked first. */}
        <Show when={page().kept > 0}>
          <MenuButton
            label={t("clipHistory.options", { name: name() })}
            items={[
              {
                icon: "remove",
                label: t("clipHistory.clear"),
                hint: t("clipHistory.clearHint", { name: name() }),
                danger: true,
                onSelect: async () => {
                  if (await askConfirm({ title: t("clipHistory.clearTitle"), body: t("clipHistory.clearHint", { name: name() }), action: t("clipHistory.clear") }))
                    clear();
                },
              },
            ]}
          >
            <Icon name="more" />
          </MenuButton>
        </Show>
      </div>
      <Show when={page().kept > 0} fallback={<p class="muted small">{t("clipHistory.empty", { name: name() })}</p>}>
        <input
          type="search"
          class="clip-log-search"
          placeholder={t("clipHistory.search")}
          aria-label={t("clipHistory.searchLabel")}
          value={query()}
          onInput={(e) => setQuery(e.currentTarget.value)}
        />
        <Show when={page().total > 0} fallback={<p class="muted small">{t("clipHistory.noMatch")}</p>}>
          <ul class="clip-log">
            <For each={page().entries}>
              {(e) => (
                <li
                  class="clip-log-row"
                  id={foundId("clipboard", e.id)}
                  tabIndex={-1}
                  classList={{ "found-target": isFound("clipboard", e.id) }}
                >
                  <span
                    class="clip-log-dir"
                    role="img"
                    classList={{ "clip-log-in": e.direction === "in" }}
                    title={way(e)}
                    aria-label={way(e)}
                  >
                    {e.direction === "in" ? "↓" : "↑"}
                  </span>
                  <Show
                    when={pictureOf(e.id)}
                    fallback={
                      <>
                        <div class="clip-log-body">
                          <span class="clip-log-text">{e.text}</span>
                          <span class="small muted">
                            {whenOf(e.at ?? 0)}
                            <Show when={e.truncated}> · {t("clipHistory.truncated")}</Show>
                          </span>
                        </div>
                        <div class="clip-log-actions">
                          <button class="btn btn-ghost btn-sm" aria-label={t("clipHistory.copyLabel")} onClick={() => copy(e.text)}>
                            {t("clipHistory.copy")}
                          </button>
                          <button
                            class="btn btn-ghost btn-sm"
                            aria-label={t("clipHistory.sendAgainLabel", { name: name() })}
                            onClick={() => sendAgain(e.text)}
                          >
                            {t("clipHistory.sendAgain")}
                          </button>
                        </div>
                      </>
                    }
                  >
                    {(pic) => (
                      <>
                        <button
                          type="button"
                          class="clip-pic-thumb"
                          aria-label={t("clipHistory.openLabel")}
                          title={t("clipHistory.openLabel")}
                          onClick={() => openPicture(pic(), e)}
                        >
                          <Show when={pic().thumb} fallback={<span aria-hidden="true">🖼</span>}>
                            {(src) => <img src={src()} alt="" />}
                          </Show>
                        </button>
                        <div class="clip-log-body">
                          <span class="clip-log-text">{t("clipHistory.picture")}</span>
                          <span class="small muted">{details(pic())}</span>
                          <span class="small muted">{whenOf(e.at ?? 0)}</span>
                        </div>
                        <div class="clip-log-actions clip-pic-actions">
                          <button
                            class="btn btn-ghost btn-sm"
                            aria-label={t("clipHistory.copyPictureLabel")}
                            onClick={() => copyPicture(e.id)}
                          >
                            {t("clipHistory.copy")}
                          </button>
                          <button
                            class="btn btn-ghost btn-sm"
                            aria-label={t("clipHistory.sendAgainLabel", { name: name() })}
                            onClick={() => sendPicture(e.id)}
                          >
                            {t("clipHistory.sendAgain")}
                          </button>
                          <button
                            class="btn btn-ghost btn-sm"
                            aria-label={t("clipHistory.saveAsLabel")}
                            onClick={() => savePicture(e.id)}
                          >
                            {t("clipHistory.saveAs")}
                          </button>
                          <button
                            class="btn btn-ghost btn-sm"
                            aria-label={t("clipHistory.openLabel")}
                            onClick={() => openPicture(pic(), e)}
                          >
                            {t("clipHistory.open")}
                          </button>
                        </div>
                      </>
                    )}
                  </Show>
                </li>
              )}
            </For>
          </ul>
          <Show when={page().total > page().entries.length}>
            <div class="row">
              <span class="small muted grow">
                {t("clipHistory.showing", { shown: page().entries.length, total: page().total })}
              </span>
              <button class="btn btn-soft btn-sm" onClick={() => setLimit((l) => l + PAGE)}>
                {t("clipHistory.more")}
              </button>
            </div>
          </Show>
        </Show>
      </Show>
      <Show when={preview()}>
        {(p) => (
          <PicturePreview
            url={p().url}
            note={`${way(p().e)} · ${details(p().pic)} · ${whenOf(p().e.at ?? 0)}`}
            alt={t("clipHistory.pictureAlt", { details: details(p().pic) })}
            onClose={() => setPreview(null)}
          />
        )}
      </Show>
    </article>
  );
}

/** A kept picture shown larger, over the page; Escape, Close or a click
 *  outside closes it, and focus goes back where it was. */
function PicturePreview(props: { url: string; note: string; alt: string; onClose: () => void }) {
  let dialog: HTMLDivElement | undefined;
  const restore = rememberFocus();
  const close = () => {
    props.onClose();
    restore();
  };
  return (
    <Portal>
      <div class="clip-pic-backdrop" onMouseDown={(e) => e.target === e.currentTarget && close()}>
        <div
          class="clip-pic-preview"
          ref={(el) => {
            dialog = el;
            queueMicrotask(() => el.querySelector<HTMLElement>("button")?.focus());
          }}
          role="dialog"
          aria-modal="true"
          aria-labelledby="clip-pic-title"
          onKeyDown={(e) => {
            if (e.key === "Escape") {
              e.preventDefault();
              close();
            } else trapTab(e, dialog);
          }}
        >
          <div class="row">
            <h2 id="clip-pic-title" class="grow">
              {t("clipHistory.picture")}
            </h2>
            <button class="btn btn-ghost btn-sm" onClick={close}>
              {t("clipHistory.close")}
            </button>
          </div>
          <img class="clip-pic-full" src={props.url} alt={props.alt} />
          <p class="small muted">{props.note}</p>
        </div>
      </div>
    </Portal>
  );
}
