// t(): look up a UI string by key and fill in `{name}` placeholders. Keys are
// typed from en.ts, so a typo is a compile error. No library: a catalogue is
// a plain object and plurals use the platform's Intl.PluralRules.
//
// Languages: English (en.ts, the source) and Hindi (hi.ts). The language
// follows the system (navigator.languages: hi* picks Hindi) unless Settings
// overrides it; the choice is remembered in localStorage. A key a language
// doesn't have yet (e.g. one added after it was translated) shows in English.

import { en, type Messages } from "./en";
import { hi } from "./hi";

/** Keys with a plain string value. */
export type Key = { [K in keyof Messages]: Messages[K] extends string ? K : never }[keyof Messages];
/** Keys with `one`/`other` plural forms. */
export type PluralKey = Exclude<keyof Messages, Key>;
export type Params = Record<string, string | number>;

/** A translation: any subset of the English keys, same shapes. */
export type Catalogue = Partial<Messages>;

export type Lang = "en" | "hi";
/** The Settings choice: follow the system, or a fixed language. */
export type LangPref = "system" | Lang;

/** Every language, named in itself (the picker shows these untranslated). */
export const LANGUAGES: { id: Lang; name: string }[] = [
  { id: "en", name: "English" },
  { id: "hi", name: "हिन्दी" },
];

const catalogues: Record<Lang, Catalogue> = { en, hi };
const STORAGE_KEY = "md.language";

let lang: Lang = "en";
let messages: Catalogue = en;
let plurals = new Intl.PluralRules(lang);

/** The language the system asks for (first of navigator.languages we have). */
export function systemLanguage(): Lang {
  const wanted = typeof navigator === "undefined" ? [] : navigator.languages?.length ? navigator.languages : [navigator.language];
  for (const tag of wanted) {
    const base = (tag || "").toLowerCase().split(/[-_]/)[0];
    if (base in catalogues) return base as Lang;
  }
  return "en";
}

/** The saved Settings choice ("system" when none). */
export function languagePref(): LangPref {
  try {
    const v = localStorage.getItem(STORAGE_KEY);
    return v === "en" || v === "hi" ? v : "system";
  } catch {
    return "system";
  }
}

/** The language in use now. */
export function language(): Lang {
  return lang;
}

/** Switch language (falls back to English for an unknown code). */
export function setLanguage(code: string) {
  lang = code in catalogues ? (code as Lang) : "en";
  messages = catalogues[lang];
  plurals = new Intl.PluralRules(lang);
  if (typeof document !== "undefined") document.documentElement.lang = lang;
}

/** Remember a Settings choice and reload the window so every screen
 *  (including strings built once at start-up) uses it. */
export function setLanguagePref(pref: LangPref) {
  try {
    if (pref === "system") localStorage.removeItem(STORAGE_KEY);
    else localStorage.setItem(STORAGE_KEY, pref);
  } catch {
    /* storage blocked: the choice lasts until the window reloads */
  }
  setLanguage(pref === "system" ? systemLanguage() : pref);
  location.reload();
}

// Pick the language before any screen renders.
{
  const pref = languagePref();
  setLanguage(pref === "system" ? systemLanguage() : pref);
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
