// A search result's text, shared by the command palette and the Search
// screen: its line with the matching words marked, and a quieter line
// saying whose it is and when.

import { For, Show } from "solid-js";
import { peerName, state } from "../lib/core";
import { highlight, type Found } from "../lib/search";
import { timeAgo } from "../lib/words";
import { t, type Key } from "../i18n";

/** `text` with the words of the query in <mark>. */
export function Marked(props: { text: string; words: string[] }) {
  return <For each={highlight(props.text, props.words)}>{(p) => (p.hit ? <mark class="found-mark">{p.s}</mark> : p.s)}</For>;
}

const TRANSFER_STATE: Record<string, Key> = {
  active: "search.transfer.active",
  done: "search.transfer.done",
  failed: "search.transfer.failed",
};

/** Whose it is and when, e.g. "From Pixel 8 · 2 min ago". */
export function foundMeta(f: Found): string {
  const parts: string[] = [];
  const name = f.node ? peerName(f.node) : "";
  switch (f.kind) {
    case "messages":
    case "clipboard":
      if (name) parts.push(t(f.tag === "out" ? "search.to" : "search.from", { name }));
      break;
    case "notifications":
      if (f.tag) parts.push(f.tag);
      if (name) parts.push(name);
      break;
    case "transfers":
      if (f.tag && TRANSFER_STATE[f.tag]) parts.push(t(TRANSFER_STATE[f.tag]));
      if (name) parts.push(name);
      break;
    case "schedules":
    case "shortcuts":
      if (f.tag) parts.push(f.tag);
      if (name) parts.push(name);
      break;
    case "notes":
      if (f.tag === "conflict") parts.push(t("notes.conflictTag"));
      // The device that last changed it (not when it was this computer).
      if (f.node && state.peers.some((p) => p.node_id === f.node)) parts.push(t("search.changedOn", { name }));
      break;
    case "settings":
      parts.push(t("nav.settings"));
      break;
    case "help":
      parts.push(t("nav.help"));
      break;
    case "devices":
      break;
    default:
      if (name) parts.push(name);
  }
  if (f.ts && f.kind !== "folders") parts.push(timeAgo(f.ts));
  if (f.ts && f.kind === "folders") parts.push(t("search.syncedAt", { when: timeAgo(f.ts) }));
  return parts.join(" · ");
}

/** Kinds whose text says more than the title (a notification's or a
 *  note's text, a file's folder). */
const WITH_DETAIL = new Set<Found["kind"]>(["notes", "notifications", "transfers", "folders"]);

/** The result's two lines. `title` empty: the text's snippet is the line. */
export function FoundText(props: { f: Found; words: string[]; class?: string }) {
  const line = () => props.f.title || props.f.detail;
  // Automations and the like: their text is a raw action name, not shown.
  const second = () => (props.f.title && WITH_DETAIL.has(props.f.kind) ? props.f.detail : "");
  return (
    <span class={`found-text ${props.class ?? ""}`}>
      <span class="found-line">
        <Marked text={line()} words={props.words} />
      </span>
      <Show when={second()}>
        <span class="found-detail small">
          <Marked text={second()} words={props.words} />
        </span>
      </Show>
      <Show when={foundMeta(props.f)}>
        <span class="found-meta small">{foundMeta(props.f)}</span>
      </Show>
    </span>
  );
}
