// Help topics as data: one Markdown file → a topic, the topics for a
// platform, and search. Pure, so it runs anywhere (topics.ts loads the
// files). The Android app does the same in core/HelpMarkdown.kt.

import { blocksText, parseBlocks, splitFrontMatter, splitList, type Block } from "./markdown";

export type Topic = {
  /** From the file name: "04-find.md" → "find". Links use `help:find`. */
  id: string;
  title: string;
  keywords: string[];
  platforms: string[];
  blocks: Block[];
  /** Lower-case text for search. */
  body: string;
  /** Sort order: the file name's number. */
  order: number;
};

/** One file → a topic, or null when it has no title (e.g. a README). */
export function parseTopic(fileName: string, src: string): Topic | null {
  const base = fileName.replace(/^.*\//, "").replace(/\.md$/i, "");
  const m = /^(\d+)-(.+)$/.exec(base);
  const id = m ? m[2] : base;
  const { meta, body } = splitFrontMatter(src);
  if (!meta.title) return null;
  const blocks = parseBlocks(body);
  return {
    id,
    title: meta.title,
    keywords: splitList(meta.keywords).map((k) => k.toLowerCase()),
    platforms: splitList(meta.platforms).map((p) => p.toLowerCase()),
    blocks,
    body: blocksText(blocks).toLowerCase(),
    order: m ? Number(m[1]) : Number.MAX_SAFE_INTEGER,
  };
}

/** Typographic and plain apostrophes match each other. */
const fold = (s: string) => s.toLowerCase().replace(/[‘’]/g, "'");

/** The words of a query, lower-case. */
export function queryWords(query: string): string[] {
  return fold(query).split(/\s+/).filter(Boolean);
}

/**
 * Topics with every word of the query somewhere in their title, keywords or
 * text, best first: a word in the title counts most, then in the keywords,
 * then in the text; ties keep the topics' own order. An empty query keeps
 * them all, in order.
 */
export function searchTopics(topics: Topic[], query: string): Topic[] {
  const words = queryWords(query);
  if (!words.length) return [...topics].sort((a, b) => a.order - b.order);
  const scored: { t: Topic; score: number }[] = [];
  for (const t of topics) {
    const title = fold(t.title);
    const keys = fold(t.keywords.join(" "));
    const body = fold(t.body);
    let score = 0;
    let all = true;
    for (const w of words) {
      const s = (title.includes(w) ? 3 : 0) + (keys.includes(w) ? 2 : 0) + (body.includes(w) ? 1 : 0);
      if (!s) {
        all = false;
        break;
      }
      score += s;
    }
    if (all) scored.push({ t, score });
  }
  return scored.sort((a, b) => b.score - a.score || a.t.order - b.t.order).map((x) => x.t);
}

/** Topics listed on [platform] ("desktop" or "android"), in order. */
export function topicsFor(topics: Topic[], platform: string): Topic[] {
  return topics.filter((t) => t.platforms.includes(platform)).sort((a, b) => a.order - b.order);
}
