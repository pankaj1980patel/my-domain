// A phone's recent photos and videos: a grid of thumbnails, newest first,
// a page at a time, each one downloadable into Downloads.

import { createEffect, createSignal, For, on, onCleanup, onMount, Show } from "solid-js";
import { createStore, reconcile } from "solid-js/store";
import { listen } from "@tauri-apps/api/event";
import { commands } from "../../bindings";
import { friendlyError, humanSize } from "../../lib/words";
import { toast } from "../../components/ui";
import { t, tn } from "../../i18n";

/** One photo or video in the listing (core `GalleryEntry`). */
type Item = {
  id: string;
  name: string;
  mime: string;
  size: number;
  taken: number;
  width: number;
  height: number;
  duration: number;
  video: boolean;
};

type Listing = { from: string; req: string; items: Item[]; next?: number | null; error?: string | null };
type Thumb = { from: string; req: string; id: string; jpeg: string };

/** Items asked for per page. */
const PAGE = 60;
/** How long to wait for a page before saying the phone didn't answer. */
const ANSWER_WAIT_MS = 30_000;

/** "1:05" for a video's length in milliseconds. */
function videoLength(ms: number): string {
  const s = Math.round(ms / 1000);
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  const ss = String(s % 60).padStart(2, "0");
  return h ? `${h}:${String(m).padStart(2, "0")}:${ss}` : `${m}:${ss}`;
}

/** The photos tile: shown for a device whose status lists "gallery". */
export default function PhotosTile(props: { node: string }) {
  const [open, setOpen] = createSignal(false);
  const [items, setItems] = createSignal<Item[]>([]);
  const [thumbs, setThumbs] = createStore<Record<string, string>>({});
  const [selected, setSelected] = createStore<Record<string, boolean>>({});
  const [next, setNext] = createSignal<number | null>(null);
  const [loading, setLoading] = createSignal(false);
  const [problem, setProblem] = createSignal<string>();
  // Our requests: answers to others (or to a page we gave up on) are ignored.
  let asked = new Set<string>();
  let pending: string | undefined;
  let timer: ReturnType<typeof setTimeout> | undefined;

  const stopWaiting = () => {
    if (timer) clearTimeout(timer);
    timer = undefined;
    pending = undefined;
    setLoading(false);
  };

  const reset = () => {
    stopWaiting();
    asked = new Set();
    setItems([]);
    setThumbs(reconcile({}));
    setSelected(reconcile({}));
    setNext(null);
    setProblem(undefined);
  };

  /** Ask for the page starting at `offset` (0: start over). */
  const load = async (offset: number) => {
    if (offset === 0) reset();
    setProblem(undefined);
    setLoading(true);
    const r = await commands.listGallery(props.node, offset, PAGE, true);
    if (r.status === "error") {
      setLoading(false);
      setProblem(friendlyError(r.error));
      return;
    }
    asked.add(r.data);
    pending = r.data;
    if (timer) clearTimeout(timer);
    timer = setTimeout(() => {
      stopWaiting();
      setProblem(t("photos.noAnswer"));
    }, ANSWER_WAIT_MS);
  };

  onMount(async () => {
    const offListing = await listen<Listing>("gallery-listing", (e) => {
      const l = e.payload;
      if (l.from !== props.node || l.req !== pending) return;
      stopWaiting();
      if (l.error) {
        // The phone says why in its own words; photo access is the usual one.
        setProblem(/access|allow|permission/i.test(l.error) ? t("photos.noAccess") : t("photos.failed", { reason: l.error }));
        return;
      }
      const have = new Set(items().map((i) => i.id));
      setItems([...items(), ...l.items.filter((i) => !have.has(i.id))]);
      setNext(l.next ?? null);
    });
    const offThumb = await listen<Thumb>("gallery-thumb", (e) => {
      const th = e.payload;
      if (th.from === props.node && asked.has(th.req)) setThumbs(th.id, `data:image/jpeg;base64,${th.jpeg}`);
    });
    onCleanup(() => {
      offListing();
      offThumb();
    });
  });
  onCleanup(stopWaiting);
  // Another device's page: start again closed.
  createEffect(on(() => props.node, () => (reset(), setOpen(false)), { defer: true }));

  const show = () => {
    setOpen(true);
    load(0);
  };

  const picked = () => items().filter((i) => selected[i.id]);
  const toggle = (id: string) => setSelected(id, !selected[id]);

  /** Fetch these items into Downloads; progress shows under Files. */
  const download = async (list: Item[]) => {
    let started = 0;
    for (const i of list) {
      const r = await commands.downloadGalleryItem(props.node, i.id, i.name);
      if (r.status === "error") {
        toast(friendlyError(r.error), "error");
        break;
      }
      started++;
    }
    if (started) toast(tn("photos.downloading", started), "ok");
  };

  const label = (i: Item) => {
    const when = i.taken ? new Date(i.taken * 1000).toLocaleString() : "";
    return [i.name, humanSize(i.size), when].filter(Boolean).join(" · ");
  };

  return (
    <article class="tile tile-wide">
      <h2>{t("photos.title")}</h2>
      <p class="muted small">{t("photos.hint")}</p>
      <Show
        when={open()}
        fallback={
          <button class="btn btn-soft btn-block" onClick={show}>
            {t("photos.show")}
          </button>
        }
      >
        <div class="row photos-bar">
          <button class="btn btn-ghost btn-sm" disabled={loading()} onClick={() => load(0)}>
            {t("photos.refresh")}
          </button>
          <Show when={picked().length}>
            <span class="muted small">{tn("photos.selected", picked().length)}</span>
            <button class="btn btn-ghost btn-sm" onClick={() => download(picked()).then(() => setSelected(reconcile({})))}>
              {t("photos.downloadSelected")}
            </button>
            <button class="btn btn-ghost btn-sm" onClick={() => setSelected(reconcile({}))}>
              {t("photos.clearSelection")}
            </button>
          </Show>
        </div>
        <Show when={problem()}>
          <p class="small text-error">{problem()}</p>
        </Show>
        <Show when={items().length}>
          <ul class="photos-grid">
            <For each={items()}>
              {(i) => (
                <li class="photo" classList={{ "photo-picked": !!selected[i.id] }}>
                  <button
                    class="photo-thumb"
                    title={label(i)}
                    aria-label={t(selected[i.id] ? "photos.unselectLabel" : "photos.selectLabel", { name: i.name })}
                    aria-pressed={!!selected[i.id]}
                    onClick={() => toggle(i.id)}
                  >
                    <Show when={thumbs[i.id]} fallback={<span class="photo-blank" aria-hidden="true" />}>
                      <img src={thumbs[i.id]} alt="" loading="lazy" />
                    </Show>
                    <Show when={i.video}>
                      <span class="photo-badge">{i.duration ? videoLength(i.duration) : t("photos.video")}</span>
                    </Show>
                    <span class="photo-check" aria-hidden="true">
                      {selected[i.id] ? "✓" : ""}
                    </span>
                  </button>
                  <button
                    class="btn btn-ghost btn-sm"
                    aria-label={t("photos.downloadLabel", { name: i.name })}
                    onClick={() => download([i])}
                  >
                    {t("photos.download")}
                  </button>
                </li>
              )}
            </For>
          </ul>
        </Show>
        <Show when={loading()}>
          <p class="muted small">{t("photos.loading")}</p>
        </Show>
        <Show when={!loading() && !problem() && !items().length}>
          <p class="muted small">{t("photos.empty")}</p>
        </Show>
        <Show when={!loading() && next() !== null}>
          <button class="btn btn-ghost btn-sm" onClick={() => load(next()!)}>
            {t("photos.more")}
          </button>
        </Show>
      </Show>
    </article>
  );
}
