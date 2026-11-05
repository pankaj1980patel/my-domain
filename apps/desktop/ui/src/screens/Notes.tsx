// Notes: a small notepad that's the same on all your devices (`lib/notes.ts`).
// The list on the left (search over titles and text, every word; pinned
// first, then the most recently changed), the open note on the right. A
// note saves itself a moment after you stop typing, and goes to your other
// devices from there. A note changed on two devices at once keeps both
// texts: the one that lost is a separate note, marked as a conflict copy.

import { createEffect, createMemo, createSignal, For, on, onCleanup, Show } from "solid-js";
import {
  addItem,
  bodyBytes,
  checklistCount,
  checklistLines,
  deleteNote,
  MAX_BODY,
  noteMatches,
  notes,
  restoreNote,
  saveNote,
  sortedNotes,
  toggleLine,
  type Note,
  type NoteFields,
} from "../lib/notes";
import { friendlyError, niceName, timeAgo } from "../lib/words";
import { Empty, toast } from "../components/ui";
import { LABEL_COLORS } from "../components/DeviceLabel";
import { HelpLink } from "../help/HelpLink";
import { t, type Key } from "../i18n";

/** How long after the last keystroke a note saves itself. */
const SAVE_AFTER_MS = 600;

/** What to open: a note (`id`), or a new one (`id` null). `at` makes the
 *  same request again open it again. */
export type NoteRequest = { id: string | null; at: number };

export default function Notes(props: { request?: NoteRequest }) {
  const [query, setQuery] = createSignal("");
  // The editor shown: `key` changes only when you pick something, so a new
  // note getting its id doesn't redraw the editor under your fingers.
  const [open, setOpen] = createSignal<{ key: number; id: string | null } | null>(null);
  // The note the editor holds (its id once a new one is saved).
  const [current, setCurrent] = createSignal<string | null>(null);
  let seq = 0;
  const show = (id: string | null) => {
    setOpen({ key: ++seq, id });
    setCurrent(id);
  };

  createEffect(
    on(
      () => props.request,
      (r) => {
        if (r) show(r.id);
      },
    ),
  );

  const list = createMemo(() => sortedNotes().filter((n) => noteMatches(n, query())));

  // Deleted on another device while open (and nothing typed here since):
  // close it.
  createEffect(() => {
    const id = current();
    if (id && notes.loaded && !notes.byId[id] && open()) {
      setOpen(null);
      setCurrent(null);
      toast(t("notes.deletedElsewhere"));
    }
  });

  const remove = async (n: Note) => {
    // Closed first, so the delete isn't taken for one from another device.
    const wasOpen = current() === n.id;
    if (wasOpen) {
      setOpen(null);
      setCurrent(null);
    }
    try {
      await deleteNote(n.id);
    } catch (e) {
      toast(friendlyError(e), "error");
      if (wasOpen) show(n.id);
      return;
    }
    toast(t("notes.deleted", { title: n.title || t("notes.untitled") }), "info", {
      label: t("notes.undo"),
      run: () => restoreNote(n).then((back) => show(back.id), (e) => toast(friendlyError(e), "error")),
    });
  };

  return (
    <section class="page notes-page">
      <header class="page-head">
        <div>
          <h1>
            {t("notes.title")} <HelpLink topic="notes" />
          </h1>
          <p class="muted">{t("notes.subtitle")}</p>
        </div>
        <button class="btn btn-primary btn-sm" onClick={() => show(null)}>
          {t("notes.new")}
        </button>
      </header>
      <div class="notes-layout">
        <div class="notes-side">
          <input
            type="search"
            class="notes-search"
            placeholder={t("notes.search")}
            aria-label={t("notes.search")}
            value={query()}
            onInput={(e) => setQuery(e.currentTarget.value)}
          />
          <Show
            when={list().length > 0}
            fallback={
              <p class="muted small notes-none">
                {Object.keys(notes.byId).length ? t("notes.noMatch", { query: query() }) : t("notes.empty")}
              </p>
            }
          >
            <ul class="notes-list" aria-label={t("notes.listLabel")}>
              <For each={list()}>
                {(n) => (
                  <li>
                    <button
                      class="note-item"
                      classList={{ "note-item-on": current() === n.id }}
                      data-color={n.color ?? undefined}
                      aria-current={current() === n.id ? "true" : undefined}
                      onClick={() => show(n.id)}
                    >
                      <span class="note-item-head">
                        <span class="note-item-title">{n.title || t("notes.untitled")}</span>
                        <Show when={n.pinned}>
                          <span class="note-pin" title={t("notes.pinned")} aria-label={t("notes.pinned")}>
                            ●
                          </span>
                        </Show>
                      </span>
                      <Show when={n.conflict_of}>
                        <span class="note-conflict-tag small">{t("notes.conflictTag")}</span>
                      </Show>
                      <span class="note-item-snippet muted small">{snippet(n)}</span>
                      <span class="note-item-when muted small">{timeAgo(n.updated_at ?? 0)}</span>
                    </button>
                  </li>
                )}
              </For>
            </ul>
          </Show>
        </div>
        <div class="notes-main">
          <Show
            when={open()}
            keyed
            fallback={
              <Empty icon="✎" title={t("notes.pickTitle")}>
                <p class="muted">{t("notes.pickBody")}</p>
              </Empty>
            }
          >
            {(o) => (
              <Editor
                id={o.id}
                onSaved={setCurrent}
                onDelete={remove}
                onOpen={(id) => show(id)}
              />
            )}
          </Show>
        </div>
      </div>
    </section>
  );
}

/** A line under the title: checklist progress, or the start of the text. */
function snippet(n: Note): string {
  if (n.checklist) {
    const [done, all] = checklistCount(n.body);
    if (all) return t("notes.progress", { done, all });
  }
  const line = n.body.split("\n").find((l) => l.trim()) ?? "";
  return line.slice(0, 120);
}

function Editor(props: {
  id: string | null;
  onSaved: (id: string) => void;
  onDelete: (n: Note) => void;
  onOpen: (id: string) => void;
}) {
  const start = props.id ? notes.byId[props.id] : undefined;
  const [id, setId] = createSignal<string | null>(props.id);
  const [title, setTitle] = createSignal(start?.title ?? "");
  const [body, setBody] = createSignal(start?.body ?? "");
  const [pinned, setPinned] = createSignal(start?.pinned ?? false);
  const [checklist, setChecklist] = createSignal(start?.checklist ?? false);
  const [color, setColor] = createSignal<string | null>(start?.color ?? null);
  const [asText, setAsText] = createSignal(false);
  const [item, setItem] = createSignal("");
  const [error, setError] = createSignal("");
  const [saving, setSaving] = createSignal(false);
  // Changed here and not saved yet.
  let dirty = false;
  // The version on screen (to spot one changed on another device).
  let shown = start?.updated_at ?? 0;
  let timer: number | undefined;
  let titleInput: HTMLInputElement | undefined;

  const note = () => {
    const i = id();
    return i ? notes.byId[i] : undefined;
  };
  const fields = (): NoteFields => ({
    title: title(),
    body: body(),
    pinned: pinned(),
    checklist: checklist(),
    color: color(),
  });
  const tooLong = () => bodyBytes(body()) > MAX_BODY;

  // One save at a time, so a new note gets its id once, not twice.
  let queue: Promise<void> = Promise.resolve();
  const save = () => {
    clearTimeout(timer);
    queue = queue.then(saveNow);
    return queue;
  };
  const saveNow = async () => {
    if (!dirty) return;
    const f = fields();
    // A new note with nothing in it isn't kept.
    if (!id() && !f.title.trim() && !f.body.trim()) return;
    if (bodyBytes(f.body) > MAX_BODY) return;
    dirty = false;
    setSaving(true);
    try {
      const saved = await saveNote(id(), f);
      shown = saved.updated_at ?? 0;
      setError("");
      if (!id()) {
        setId(saved.id);
        props.onSaved(saved.id);
      }
    } catch (e) {
      dirty = true;
      setError(friendlyError(e));
    } finally {
      setSaving(false);
    }
  };
  const changed = () => {
    dirty = true;
    clearTimeout(timer);
    timer = window.setTimeout(save, SAVE_AFTER_MS);
  };
  /** Pin, colour, checklist: saved at once. */
  const now = () => {
    dirty = true;
    void save();
  };
  onCleanup(() => {
    // Leaving with something typed: save it now.
    if (dirty) void save();
  });

  // Changed on another device: show it, unless you're typing here.
  createEffect(
    on(note, (n) => {
      if (!n || dirty || (n.updated_at ?? 0) === shown) return;
      shown = n.updated_at ?? 0;
      setTitle(n.title);
      setBody(n.body);
      setPinned(n.pinned);
      setChecklist(n.checklist);
      setColor(n.color);
    }),
  );

  // A new note starts in the title.
  if (!props.id) queueMicrotask(() => titleInput?.focus());

  const lines = createMemo(() => checklistLines(body()));
  const toggle = (i: number) => {
    setBody(toggleLine(body(), i));
    now();
  };
  const add = (e: Event) => {
    e.preventDefault();
    if (!item().trim()) return;
    setBody(addItem(body(), item()));
    setItem("");
    now();
  };

  const original = () => {
    const of = note()?.conflict_of;
    return of ? notes.byId[of] : undefined;
  };

  return (
    <article class="note-editor" data-color={color() ?? undefined}>
      <Show when={note()?.conflict_of}>
        <div class="note-conflict" role="note">
          <strong>{t("notes.conflictTitle")}</strong>
          <p class="small">{t("notes.conflictBody", { name: niceName(note()?.by_name || t("notes.anotherDevice")) })}</p>
          <Show when={original()}>
            {(o) => (
              <button class="btn btn-ghost btn-sm" onClick={() => props.onOpen(o().id)}>
                {t("notes.openOriginal", { title: o().title || t("notes.untitled") })}
              </button>
            )}
          </Show>
        </div>
      </Show>
      <input
        ref={titleInput}
        class="note-title-input"
        type="text"
        maxLength={200}
        placeholder={t("notes.titlePlaceholder")}
        aria-label={t("notes.titleLabel")}
        value={title()}
        onInput={(e) => {
          setTitle(e.currentTarget.value);
          changed();
        }}
        onBlur={() => void save()}
      />
      <div class="note-tools" role="toolbar" aria-label={t("notes.tools")}>
        <button
          class="btn btn-ghost btn-sm"
          aria-pressed={pinned()}
          onClick={() => {
            setPinned(!pinned());
            now();
          }}
        >
          {pinned() ? t("notes.unpin") : t("notes.pin")}
        </button>
        <button
          class="btn btn-ghost btn-sm"
          aria-pressed={checklist()}
          onClick={() => {
            setChecklist(!checklist());
            setAsText(false);
            now();
          }}
        >
          {t("notes.checklist")}
        </button>
        <div class="note-colors" role="radiogroup" aria-label={t("notes.color")}>
          <button
            class="label-swatch label-swatch-none note-swatch"
            classList={{ on: !color() }}
            role="radio"
            aria-checked={!color()}
            aria-label={t("notes.noColor")}
            title={t("notes.noColor")}
            onClick={() => {
              setColor(null);
              now();
            }}
          />
          <For each={LABEL_COLORS}>
            {(c) => (
              <button
                class="label-swatch note-swatch"
                classList={{ on: color() === c }}
                data-color={c}
                role="radio"
                aria-checked={color() === c}
                aria-label={t(`label.color.${c}` as Key)}
                title={t(`label.color.${c}` as Key)}
                onClick={() => {
                  setColor(c);
                  now();
                }}
              />
            )}
          </For>
        </div>
        <Show when={note()}>
          {(n) => (
            <button
              class="btn btn-ghost btn-sm note-delete"
              onClick={() => {
                // What's on screen is what Undo brings back; nothing is
                // saved on the way out.
                const last = { ...n(), ...fields() };
                dirty = false;
                clearTimeout(timer);
                props.onDelete(last);
              }}
            >
              {t("notes.delete")}
            </button>
          )}
        </Show>
      </div>
      <Show
        when={checklist() && !asText()}
        fallback={
          <textarea
            class="note-body"
            placeholder={t("notes.bodyPlaceholder")}
            aria-label={t("notes.bodyLabel")}
            value={body()}
            onInput={(e) => {
              setBody(e.currentTarget.value);
              changed();
            }}
            onBlur={() => void save()}
          />
        }
      >
        <ul class="note-checklist" aria-label={t("notes.bodyLabel")}>
          <For each={lines()}>
            {(l, i) => (
              <Show when={l.box} fallback={<li class="note-line">{l.text}</li>}>
                <li class="note-check" classList={{ "note-check-done": l.checked }}>
                  <label>
                    <input type="checkbox" checked={l.checked} onChange={() => toggle(i())} />
                    <span>{l.text}</span>
                  </label>
                </li>
              </Show>
            )}
          </For>
        </ul>
        <form class="note-add" onSubmit={add}>
          <input
            type="text"
            placeholder={t("notes.addItem")}
            aria-label={t("notes.addItem")}
            value={item()}
            onInput={(e) => setItem(e.currentTarget.value)}
          />
          <button class="btn btn-soft btn-sm" type="submit">
            {t("notes.add")}
          </button>
        </form>
      </Show>
      <Show when={checklist()}>
        <button class="btn btn-ghost btn-sm note-as-text" onClick={() => setAsText(!asText())}>
          {asText() ? t("notes.asList") : t("notes.asText")}
        </button>
      </Show>
      <Show when={checklist() && asText()}>
        <p class="muted small">{t("notes.checklistHint")}</p>
      </Show>
      <Show when={tooLong()}>
        <p class="note-error small" role="alert">
          {t("notes.tooLong")}
        </p>
      </Show>
      <Show when={error()}>
        <p class="note-error small" role="alert">
          {error()}
        </p>
      </Show>
      <p class="muted small note-foot" aria-live="polite">
        <Show when={note()} fallback={t("notes.notSaved")}>
          {(n) =>
            saving()
              ? t("notes.saving")
              : t("notes.savedBy", { name: niceName(n().by_name || t("notes.anotherDevice")), when: timeAgo(n().updated_at ?? 0) })
          }
        </Show>
      </p>
    </article>
  );
}
