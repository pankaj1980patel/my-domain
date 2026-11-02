// t(): look up a UI string by key and fill in `{name}` placeholders. Keys are
// typed from en.ts, so a typo is a compile error. No library: a catalogue is
// a plain object and plurals use the platform's Intl.PluralRules.

import { en, type Messages } from "./en";

/** Keys with a plain string value. */
export type Key = { [K in keyof Messages]: Messages[K] extends string ? K : never }[keyof Messages];
/** Keys with `one`/`other` plural forms. */
export type PluralKey = Exclude<keyof Messages, Key>;
export type Params = Record<string, string | number>;

const catalogues: Record<string, Messages> = { en };
let lang = "en";
let messages: Messages = en;
let plurals = new Intl.PluralRules(lang);

/** Switch language (falls back to English for an unknown code). */
export function setLanguage(code: string) {
  lang = catalogues[code] ? code : "en";
  messages = catalogues[lang];
  plurals = new Intl.PluralRules(lang);
}

function fill(s: string, params?: Params): string {
  if (!params) return s;
  return s.replace(/\{(\w+)\}/g, (m, k: string) => (k in params ? String(params[k]) : m));
}

/** A UI string, e.g. t("activity.connected", { name }). */
export function t(key: Key, params?: Params): string {
  return fill(messages[key] ?? en[key], params);
}

/** A counted string: picks the plural form for `count` and fills `{count}`. */
export function tn(key: PluralKey, count: number, params?: Params): string {
  const forms = messages[key] ?? en[key];
  const form = plurals.select(count) === "one" ? forms.one : forms.other;
  return fill(form, { ...params, count });
}

/** A string with rich parts (e.g. a <strong> name) dropped into its
 *  placeholders: returns the text pieces and parts in order, ready for JSX. */
export function tRich<T>(key: Key, parts: Record<string, T>): (string | T)[] {
  const out: (string | T)[] = [];
  let last = 0;
  const s = messages[key] ?? en[key];
  for (const m of s.matchAll(/\{(\w+)\}/g)) {
    if (!(m[1] in parts)) continue;
    if (m.index! > last) out.push(s.slice(last, m.index));
    out.push(parts[m[1]]);
    last = m.index! + m[0].length;
  }
  if (last < s.length) out.push(s.slice(last));
  return out;
}
