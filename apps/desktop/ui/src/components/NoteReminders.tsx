// Note reminders in the window (`lib/notes.ts`): a reminder that came due
// here stays up, with Open, Done and Snooze, until one of those or until
// the note is Done or given a new time on any device. The OS notification
// says the same, but can't carry buttons on every OS (and a click on it
// doesn't reach the app), so these are where you act on it.

import { createEffect, createMemo, For, Show } from "solid-js";
import {
  dismissReminder,
  notes,
  reminderDone,
  reminderDue,
  reminderTime,
  setReminder,
  shownReminders,
  snoozeUntil,
  type Note,
  type Snooze,
} from "../lib/notes";
import { friendlyError } from "../lib/words";
import { toast } from "./ui";
import { t } from "../i18n";

/** A small bell (notes with a reminder). */
export function Bell(props: { class?: string; label?: string }) {
  return (
    <svg
      class={`bell ${props.class ?? ""}`}
      viewBox="0 0 16 16"
      width="14"
      height="14"
      role={props.label ? "img" : undefined}
      aria-label={props.label}
      aria-hidden={props.label ? undefined : "true"}
    >
      <path
        d="M8 1.5a4 4 0 0 0-4 4v2.6L2.6 10.6a.6.6 0 0 0 .5.9h9.8a.6.6 0 0 0 .5-.9L12 8.1V5.5a4 4 0 0 0-4-4Zm-1.6 11.3a1.7 1.7 0 0 0 3.2 0Z"
        fill="currentColor"
      />
    </svg>
  );
}

/** The first line of the text that isn't blank (checklist boxes left out). */
export function firstLine(body: string): string {
  const line = body.split("\n").map((l) => l.trim()).find(Boolean) ?? "";
  return line.replace(/^\[( |x|X)\] /, "").slice(0, 200);
}

/** The snooze buttons: 10 min, 1 hour, tomorrow 9:00. */
export function SnoozeButtons(props: { onSnooze: (s: Snooze) => void }) {
  const options: [Snooze, string][] = [
    ["10m", t("notes.remind.snooze10m")],
    ["1h", t("notes.remind.snooze1h")],
    ["tomorrow", t("notes.remind.snoozeTomorrow")],
  ];
  return (
    <span class="snooze-buttons" role="group" aria-label={t("notes.remind.snooze")}>
      <span class="muted small">{t("notes.remind.snooze")}</span>
      <For each={options}>
        {([s, label]) => (
          <button class="btn btn-ghost btn-sm" onClick={() => props.onSnooze(s)}>
            {label}
          </button>
        )}
      </For>
    </span>
  );
}

export async function snooze(n: Note, s: Snooze) {
  try {
    const saved = await setReminder(n.id, snoozeUntil(s, new Date()));
    toast(t("notes.remind.snoozed", { when: reminderTime(saved.remind_at ?? 0) }));
  } catch (e) {
    toast(friendlyError(e), "error");
  }
}

export async function done(n: Note) {
  try {
    await reminderDone(n.id);
  } catch (e) {
    toast(friendlyError(e), "error");
  }
}

export default function NoteReminders(props: { onOpen: (id: string) => void }) {
  // Still due: not Done, and not given a later time since.
  const due = createMemo(() =>
    shownReminders.ids
      .map((id) => notes.byId[id])
      .filter((n): n is Note => !!n && reminderDue(n, Date.now())),
  );
  // Done, snoozed or deleted (here or elsewhere): off the window.
  createEffect(() => {
    const keep = new Set(due().map((n) => n.id));
    for (const id of shownReminders.ids) if (!keep.has(id)) dismissReminder(id);
  });

  return (
    <Show when={due().length > 0}>
      <section class="reminder-stack" aria-label={t("notes.remind.region")}>
        <For each={due()}>
          {(n) => (
            <div class="reminder-card" role="alert">
              <div class="reminder-head">
                <Bell />
                <strong class="reminder-title">{n.title || t("notes.untitled")}</strong>
                <button
                  class="btn btn-ghost btn-sm reminder-close"
                  aria-label={t("notes.remind.dismiss")}
                  title={t("notes.remind.dismiss")}
                  onClick={() => dismissReminder(n.id)}
                >
                  ×
                </button>
              </div>
              <Show when={firstLine(n.body)}>
                <p class="small reminder-line">{firstLine(n.body)}</p>
              </Show>
              <p class="muted small">{reminderTime(n.remind_at ?? 0)}</p>
              <div class="reminder-actions">
                <button class="btn btn-primary btn-sm" onClick={() => void done(n)}>
                  {t("notes.remind.done")}
                </button>
                <button
                  class="btn btn-soft btn-sm"
                  onClick={() => {
                    dismissReminder(n.id);
                    props.onOpen(n.id);
                  }}
                >
                  {t("notes.remind.open")}
                </button>
              </div>
              <SnoozeButtons onSnooze={(s) => void snooze(n, s)} />
            </div>
          )}
        </For>
      </section>
    </Show>
  );
}
