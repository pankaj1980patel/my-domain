// Help topics: the Markdown files in docs/help (one source for both apps),
// bundled at build time, parsed once and filtered to this platform.

import { parseTopic, topicsFor, type Topic } from "./search";

export type { Topic } from "./search";

// docs/help is outside the UI's folder: vite.config allows it for the dev
// server; a build reads it directly.
const files = import.meta.glob("../../../../../docs/help/*.md", { query: "?raw", import: "default", eager: true }) as Record<
  string,
  string
>;

/** This app's topics (desktop), in order. */
export const TOPICS: Topic[] = topicsFor(
  Object.entries(files)
    .map(([path, src]) => parseTopic(path, src))
    .filter((t): t is Topic => t !== null),
  "desktop",
);

export function topicById(id: string | undefined): Topic | undefined {
  return id ? TOPICS.find((t) => t.id === id) : undefined;
}
