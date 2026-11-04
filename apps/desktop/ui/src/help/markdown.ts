// A small Markdown subset for the Help topics (docs/help/*.md), parsed into
// plain data so the page renders it without innerHTML. Supported: front
// matter (`key: value` between `---` lines), `#`..`###` headings,
// paragraphs, one level of `-`/`*` or `1.` lists, **bold**, *italic* or
// _italic_, `code` and [links](target). Anything else is plain text.
// The Android app has the same parser in core/HelpMarkdown.kt.

export type Inline =
  | { kind: "text"; text: string }
  | { kind: "bold"; children: Inline[] }
  | { kind: "italic"; children: Inline[] }
  | { kind: "code"; text: string }
  | { kind: "link"; href: string; children: Inline[] };

export type Block =
  | { kind: "heading"; level: number; inlines: Inline[] }
  | { kind: "para"; inlines: Inline[] }
  | { kind: "list"; ordered: boolean; items: Inline[][] };

/** A file split into its front matter (lower-case keys) and body. */
export function splitFrontMatter(src: string): { meta: Record<string, string>; body: string } {
  const text = src.replace(/^﻿/, "").replace(/\r\n?/g, "\n");
  const meta: Record<string, string> = {};
  if (!text.startsWith("---\n")) return { meta, body: text };
  const end = text.indexOf("\n---", 3);
  if (end < 0) return { meta, body: text };
  for (const line of text.slice(4, end).split("\n")) {
    const i = line.indexOf(":");
    if (i <= 0) continue;
    meta[line.slice(0, i).trim().toLowerCase()] = line.slice(i + 1).trim();
  }
  const after = text.indexOf("\n", end + 4);
  return { meta, body: after < 0 ? "" : text.slice(after + 1) };
}

/** "a, b ,c" → ["a", "b", "c"] (empty parts dropped). */
export function splitList(value: string | undefined): string[] {
  return (value ?? "")
    .split(",")
    .map((s) => s.trim())
    .filter(Boolean);
}

const HEADING = /^(#{1,6})\s+(.*)$/;
const BULLET = /^\s*[-*+]\s+(.*)$/;
const NUMBERED = /^\s*\d+[.)]\s+(.*)$/;

/** Markdown body → blocks. */
export function parseBlocks(body: string): Block[] {
  const lines = body.replace(/\r\n?/g, "\n").split("\n");
  const out: Block[] = [];
  let para: string[] = [];
  let list: { ordered: boolean; items: string[] } | null = null;

  const flushPara = () => {
    if (para.length) out.push({ kind: "para", inlines: parseInline(para.join(" ")) });
    para = [];
  };
  const flushList = () => {
    if (list) out.push({ kind: "list", ordered: list.ordered, items: list.items.map(parseInline) });
    list = null;
  };

  for (const raw of lines) {
    const line = raw.trimEnd();
    if (!line.trim()) {
      flushPara();
      flushList();
      continue;
    }
    const h = HEADING.exec(line);
    if (h) {
      flushPara();
      flushList();
      out.push({ kind: "heading", level: Math.min(h[1].length, 3), inlines: parseInline(h[2].trim()) });
      continue;
    }
    const b = BULLET.exec(line);
    const n = b ? null : NUMBERED.exec(line);
    if (b || n) {
      flushPara();
      const ordered = !!n;
      if (list && list.ordered !== ordered) flushList();
      if (!list) list = { ordered, items: [] };
      list.items.push((b ?? n)![1].trim());
      continue;
    }
    if (list) {
      // An indented line carries on the item above; anything else ends the list.
      if (/^\s/.test(raw)) {
        list.items[list.items.length - 1] += " " + line.trim();
        continue;
      }
      flushList();
    }
    para.push(line.trim());
  }
  flushPara();
  flushList();
  return out;
}

/** One line of text → inline pieces. Unclosed markers stay as text. */
export function parseInline(src: string): Inline[] {
  const out: Inline[] = [];
  let text = "";
  const push = (node: Inline) => {
    if (text) out.push({ kind: "text", text });
    text = "";
    out.push(node);
  };
  let i = 0;
  while (i < src.length) {
    const c = src[i];
    if (c === "\\" && i + 1 < src.length && /[\\`*_[\]()#+\-.!]/.test(src[i + 1])) {
      text += src[i + 1];
      i += 2;
      continue;
    }
    if (c === "`") {
      const end = src.indexOf("`", i + 1);
      if (end > i + 1) {
        push({ kind: "code", text: src.slice(i + 1, end) });
        i = end + 1;
        continue;
      }
    }
    if (src.startsWith("**", i) || src.startsWith("__", i)) {
      const mark = src.slice(i, i + 2);
      const end = src.indexOf(mark, i + 2);
      if (end > i + 2) {
        push({ kind: "bold", children: parseInline(src.slice(i + 2, end)) });
        i = end + 2;
        continue;
      }
    }
    if ((c === "*" || c === "_") && i + 1 < src.length && src[i + 1] !== " ") {
      // `_` only as a word edge, so snake_case words stay as they are.
      const wordBefore = i > 0 && /\w/.test(src[i - 1]);
      if (c === "*" || !wordBefore) {
        let end = src.indexOf(c, i + 1);
        while (end > 0 && c === "_" && end + 1 < src.length && /\w/.test(src[end + 1])) end = src.indexOf(c, end + 1);
        if (end > i + 1 && src[end - 1] !== " ") {
          push({ kind: "italic", children: parseInline(src.slice(i + 1, end)) });
          i = end + 1;
          continue;
        }
      }
    }
    if (c === "[") {
      const close = src.indexOf("]", i + 1);
      if (close > i && src[close + 1] === "(") {
        const end = src.indexOf(")", close + 2);
        if (end > close) {
          push({ kind: "link", href: src.slice(close + 2, end).trim(), children: parseInline(src.slice(i + 1, close)) });
          i = end + 1;
          continue;
        }
      }
    }
    text += c;
    i++;
  }
  if (text) out.push({ kind: "text", text });
  return out;
}

/** The words of some inline pieces, as plain text. */
export function inlineText(inlines: Inline[]): string {
  return inlines
    .map((n) => (n.kind === "text" || n.kind === "code" ? n.text : inlineText(n.children)))
    .join("");
}

/** All the text of some blocks, one block per line. */
export function blocksText(blocks: Block[]): string {
  return blocks
    .map((b) => (b.kind === "list" ? b.items.map(inlineText).join("\n") : inlineText(b.inlines)))
    .join("\n");
}

/** The topic a `help:` link points to, or null for any other link. */
export function helpTarget(href: string): string | null {
  return href.startsWith("help:") ? href.slice(5).trim() || null : null;
}
