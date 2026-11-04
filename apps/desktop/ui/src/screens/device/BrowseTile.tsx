// Browse another device's shared folders and download files from them.
// Only shown when the device says it can be browsed (`shared_folders` in
// its status). The listing is asked for with listFolder and answered by a
// `folder-listing` event carrying the same request id.

import { createEffect, createSignal, For, on, onCleanup, onMount, Show } from "solid-js";
import { listen } from "@tauri-apps/api/event";
import { commands } from "../../bindings";
import { peerName, setState, state, unwrap } from "../../lib/core";
import { friendlyError, humanSize, niceName } from "../../lib/words";
import { toast } from "../../components/ui";
import { t } from "../../i18n";

/** One item in a listing (mdcore `RemoteEntry`): `modified` is unix seconds, 0 if unknown. */
type Entry = { name: string; dir: boolean; size: number; modified: number };

/** The `folder-listing` event: one page of a folder, or why there's none. */
type Listing = {
  from: string;
  req: string;
  root: string;
  path: string;
  entries: Entry[];
  next?: number | null;
  error?: string | null;
};

export default function BrowseTile(props: { node: string }) {
  const shown = () => state.status[props.node]?.sharedFolders !== undefined;
  return (
    <Show when={shown()}>
      <Browser node={props.node} />
    </Show>
  );
}

function Browser(props: { node: string }) {
  const [open, setOpen] = createSignal(false);
  // Where we are: `root` "" is the list of shared folders, `path` is inside it.
  const [root, setRoot] = createSignal("");
  const [path, setPath] = createSignal("");
  const [entries, setEntries] = createSignal<Entry[]>([]);
  const [next, setNext] = createSignal<number>();
  const [error, setError] = createSignal<string>();
  const [loading, setLoading] = createSignal(false);
  // Downloads started here, by file location, to show them as under way.
  const [downloads, setDownloads] = createSignal<Record<string, string>>({});
  const name = () => niceName(peerName(props.node));

  // The request we're waiting for; replies to older ones are dropped. A
  // reply can beat listFolder's answer, so unclaimed ones wait in `early`.
  let want: string | undefined;
  let append = false;
  const early = new Map<string, Listing>();

  const take = (l: Listing) => {
    want = undefined;
    setLoading(false);
    if (l.error) {
      setError(l.error);
      setEntries([]);
      setNext(undefined);
      return;
    }
    setError(undefined);
    setEntries((cur) => (append ? [...cur, ...(l.entries ?? [])] : (l.entries ?? [])));
    setNext(l.next ?? undefined);
  };

  const ask = async (r: string, p: string, offset = 0) => {
    want = undefined;
    append = offset > 0;
    setLoading(true);
    if (!append) {
      setRoot(r);
      setPath(p);
      setEntries([]);
      setNext(undefined);
      setError(undefined);
    }
    try {
      const req = await unwrap(commands.listFolder(props.node, r, p, offset));
      want = req;
      const got = early.get(req);
      early.clear();
      if (got) take(got);
    } catch (e) {
      setLoading(false);
      toast(friendlyError(e), "error");
    }
  };

  onMount(async () => {
    const unlisten = await listen<Listing>("folder-listing", (e) => {
      const l = e.payload;
      if (l.from !== props.node) return;
      if (l.req === want) take(l);
      else if (!want) early.set(l.req, l);
    });
    onCleanup(unlisten);
  });

  // Another device's page: start over.
  createEffect(
    on(
      () => props.node,
      () => {
        want = undefined;
        setOpen(false);
        setRoot("");
        setPath("");
        setEntries([]);
        setDownloads({});
      },
      { defer: true },
    ),
  );

  const start = () => {
    setOpen(true);
    ask("", "");
  };

  /** The folders above the current one, for the path line. */
  const crumbs = () => {
    if (!root()) return [];
    const parts = path() ? path().split("/") : [];
    return [
      { label: root(), path: "" },
      ...parts.map((part, i) => ({ label: part, path: parts.slice(0, i + 1).join("/") })),
    ];
  };

  const up = () => {
    if (!root()) return;
    if (!path()) return ask("", "");
    ask(root(), path().split("/").slice(0, -1).join("/"));
  };

  const inside = (e: Entry) => (root() ? (path() ? `${path()}/${e.name}` : e.name) : "");
  const openFolder = (e: Entry) => (root() ? ask(root(), inside(e)) : ask(e.name, ""));
  const key = (file: string) => `${root()}/${file}`;

  const busy = (e: Entry) => {
    const id = downloads()[key(inside(e))];
    const x = id ? state.transfers[id] : undefined;
    return !!x && (x.state === "waiting" || x.state === "receiving");
  };

  const download = async (e: Entry) => {
    const file = inside(e);
    try {
      const id = await unwrap(commands.downloadFile(props.node, root(), file));
      if (!state.transfers[id]) {
        setState("transfers", id, {
          id,
          node: props.node,
          name: e.name,
          done: 0,
          total: e.size,
          incoming: true,
          state: "waiting",
          ts: Date.now(),
        });
      }
      setDownloads((d) => ({ ...d, [key(file)]: id }));
      toast(t("browse.started", { file: e.name, name: name() }), "ok");
    } catch (err) {
      toast(friendlyError(err), "error");
    }
  };

  /** The device's reason, in plain words. */
  const problem = () => {
    const r = (error() ?? "").toLowerCase();
    if (r.includes("isn't shared")) return t("browse.error.notShared", { name: name() });
    if (r.includes("missing")) return t("browse.error.missing", { name: name() });
    if (r.includes("not found") || r.includes("not a folder")) return t("browse.error.notFound");
    if (r.includes("can't read") || r.includes("permission")) return t("browse.error.cantRead", { name: name() });
    return t("browse.error.other", { name: name(), reason: error() ?? "" });
  };

  const date = (secs: number) =>
    secs ? new Date(secs * 1000).toLocaleDateString(undefined, { year: "numeric", month: "short", day: "numeric" }) : "";
  const info = (e: Entry) => {
    const d = date(e.modified);
    return d ? t("browse.fileInfo", { size: humanSize(e.size), date: d }) : humanSize(e.size);
  };

  return (
    <article class="tile tile-wide">
      <div class="tile-head">
        <h2>{t("browse.title")}</h2>
        <Show when={open()}>
          <div class="row">
            <button class="btn btn-ghost btn-sm" disabled={!root() || loading()} onClick={up} aria-label={t("browse.upLabel")}>
              {t("browse.up")}
            </button>
            <button class="btn btn-ghost btn-sm" onClick={() => setOpen(false)}>
              {t("browse.close")}
            </button>
          </div>
        </Show>
      </div>
      <p class="muted small">{t("browse.hint", { name: name() })}</p>
      <Show
        when={open()}
        fallback={
          <button class="btn btn-soft" onClick={start}>
            {t("browse.open", { name: name() })}
          </button>
        }
      >
        <nav class="browse-crumbs small" aria-label={t("browse.crumbsLabel")}>
          <button class="link" disabled={!root()} onClick={() => ask("", "")}>
            {t("browse.top")}
          </button>
          <For each={crumbs()}>
            {(c, i) => (
              <>
                <span class="muted" aria-hidden="true">
                  /
                </span>
                <button class="link" disabled={i() === crumbs().length - 1} onClick={() => ask(root(), c.path)}>
                  {c.label}
                </button>
              </>
            )}
          </For>
        </nav>
        <Show
          when={!error()}
          fallback={
            <div class="stack">
              <p class="small text-error">{problem()}</p>
              <div class="row">
                <button class="btn btn-ghost btn-sm" onClick={() => ask(root(), path())}>
                  {t("browse.retry")}
                </button>
              </div>
            </div>
          }
        >
          <ul class="browse-list">
            <For each={entries()}>
              {(e) => (
                <li class="browse-row">
                  <Show
                    when={e.dir}
                    fallback={
                      <>
                        <div class="browse-name grow">
                          <span class="browse-file">{e.name}</span>
                          <span class="muted small">{info(e)}</span>
                        </div>
                        <button
                          class="btn btn-ghost btn-sm"
                          disabled={busy(e)}
                          onClick={() => download(e)}
                          aria-label={t("browse.downloadLabel", { file: e.name })}
                        >
                          {busy(e) ? t("browse.downloading") : t("browse.download")}
                        </button>
                      </>
                    }
                  >
                    <button
                      class="browse-folder grow"
                      onClick={() => openFolder(e)}
                      aria-label={t("browse.openLabel", { folder: e.name })}
                    >
                      <span class="browse-file">{e.name}</span>
                      <span class="muted small">{date(e.modified) || t("browse.folder")}</span>
                    </button>
                  </Show>
                </li>
              )}
            </For>
          </ul>
          <Show when={loading()}>
            <p class="muted small">{t("browse.loading", { name: name() })}</p>
          </Show>
          <Show when={!loading() && entries().length === 0}>
            <p class="muted small">{root() ? t("browse.empty") : t("browse.nothingShared", { name: name() })}</p>
          </Show>
          <Show when={!loading() && next() !== undefined}>
            <button class="btn btn-ghost btn-sm" onClick={() => ask(root(), path(), next())}>
              {t("browse.loadMore")}
            </button>
          </Show>
        </Show>
      </Show>
    </article>
  );
}
