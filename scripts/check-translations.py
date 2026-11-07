#!/usr/bin/env python3
"""Check the apps' translations against English.

Android: every translatable <string>/<plurals> in res/values/strings.xml must
exist in each res/values-XX/strings.xml with the same format args (%1$s, %d,
%%), the same plural structure, no translatable="false" entry, no unknown
name, and no bare apostrophe or double quote (aapt rejects or mangles them).

Desktop: every key in ui/src/i18n/en.ts must exist in each other catalogue
(hi.ts, ...) with the same {placeholders}, and plurals must stay plurals.
(Unknown keys and wrong shapes are already tsc errors: catalogues are typed
Partial<Messages>.)

Both: every language found (values-XX, i18n/XX.ts) must be offered — listed in
res/xml/locales_config.xml, and imported + in LANGUAGES in i18n/index.ts.

Usage: scripts/check-translations.py [--allow-missing]
  --allow-missing  report keys a language lacks (they show in English) as
                   warnings, not errors; other problems still fail.
Exit status 0 when clean, 1 otherwise.

CI (.github/workflows/ci.yml, desktop UI job) runs it with --allow-missing,
so a new English string doesn't block a feature PR on its translation: missing
keys are only warnings there, while format-arg/placeholder mismatches, bad
plurals, bare quotes and XML that doesn't parse still fail the build. Run it
without the flag now and then and catch the translations up until it ends
with 0 errors.
"""

import re
import sys
import xml.etree.ElementTree as ET
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
RES = ROOT / "apps/android/app/src/main/res"
I18N = ROOT / "apps/desktop/ui/src/i18n"

errors: list[str] = []
warnings: list[str] = []


def missing(msg: str, allow: bool) -> None:
    (warnings if allow else errors).append(msg)


# ---------------------------------------------------------------- Android

FMT = re.compile(r"%(?:\d+\$)?[-#+ 0,(]*\d*(?:\.\d+)?[sdfxXc]|%%")


def android_args(text: str) -> list[str]:
    return sorted(FMT.findall(text or ""))


def element_text(el: ET.Element) -> str:
    return "".join(el.itertext())


def load_android(path: Path) -> tuple[dict, set]:
    """name -> ("string", text) | ("plurals", {quantity: text}); plus the
    names marked translatable="false"."""
    tree = ET.parse(path)
    out: dict = {}
    untranslatable: set = set()
    for el in tree.getroot():
        name = el.get("name")
        if not name:
            continue
        if el.get("translatable") == "false":
            untranslatable.add(name)
            continue
        if el.tag == "string":
            out[name] = ("string", element_text(el))
        elif el.tag == "plurals":
            out[name] = ("plurals", {i.get("quantity"): element_text(i) for i in el.findall("item")})
    return out, untranslatable


def check_android(allow_missing: bool) -> None:
    base_path = RES / "values/strings.xml"
    base, base_untranslatable = load_android(base_path)
    for path in sorted(RES.glob("values-*/strings.xml")):
        lang = path.parent.name.removeprefix("values-")
        try:
            tr, tr_untranslatable = load_android(path)
        except ET.ParseError as e:
            errors.append(f"android {lang}: {path} does not parse: {e}")
            continue
        raw = path.read_text(encoding="utf-8")
        for n in sorted(tr_untranslatable | (set(tr) & base_untranslatable)):
            errors.append(f"android {lang}: {n} is translatable=\"false\" in English; leave it out")
        for n in sorted(set(tr) - set(base) - base_untranslatable):
            errors.append(f"android {lang}: {n} is not in values/strings.xml")
        for n in sorted(set(base) - set(tr)):
            missing(f"android {lang}: missing {n} (shows in English)", allow_missing)
        for n, (kind, val) in base.items():
            if n not in tr:
                continue
            tkind, tval = tr[n]
            if tkind != kind:
                errors.append(f"android {lang}: {n} is a {tkind}, English has a {kind}")
                continue
            if kind == "string":
                if android_args(val) != android_args(tval):
                    errors.append(f"android {lang}: {n} format args {android_args(tval)} != English {android_args(val)}")
            else:
                if "other" not in tval:
                    errors.append(f"android {lang}: {n} has no quantity=\"other\"")
                en_all = set(a for v in val.values() for a in android_args(v))
                for q, text in tval.items():
                    extra = set(android_args(text)) - en_all
                    if extra:
                        errors.append(f"android {lang}: {n}[{q}] has args {sorted(extra)} English never uses")
                if "other" in tval and "other" in val and android_args(val["other"]) != android_args(tval["other"]):
                    errors.append(f"android {lang}: {n}[other] format args differ from English")
        # Bare quotes inside element text: aapt drops ' and " unless escaped.
        for m in re.finditer(r">([^<]*)<", raw):
            text = m.group(1)
            if re.search(r"(?<!\\)'", text) or re.search(r'(?<!\\)"', text):
                line = raw.count("\n", 0, m.start()) + 1
                errors.append(f"android {lang}: line {line}: bare ' or \" in text (use ’ or “ ”)")


# ---------------------------------------------------------------- Desktop

TOKEN = re.compile(
    r"""\s+|//[^\n]*|/\*.*?\*/|"(?:\\.|[^"\\])*"|'(?:\\.|[^'\\])*'|[A-Za-z_$][\w$]*|[{}:,]|.""",
    re.S,
)
PLACEHOLDER = re.compile(r"\{(\w+)\}")


def ts_string(tok: str) -> str:
    """A TS string literal's text (\\uXXXX, \\n, \\t and \\<char>)."""
    esc = {"n": "\n", "t": "\t", "r": "\r"}

    def one(m: re.Match) -> str:
        if m.group(1):
            return chr(int(m.group(1), 16))
        return esc.get(m.group(2), m.group(2))

    return re.sub(r"\\(?:u([0-9a-fA-F]{4})|(.))", one, tok[1:-1], flags=re.S)


def load_ts(path: Path, name: str) -> dict:
    """The `export const <name> ... = { ... }` object: key -> str | {form: str}."""
    src = path.read_text(encoding="utf-8")
    m = re.search(rf"export const {name}\b[^=]*=\s*\{{", src)
    if not m:
        raise ValueError(f"{path}: no `export const {name} = {{`")
    toks = [t for t in TOKEN.findall(src, m.end()) if not t.isspace() and not t.startswith(("//", "/*"))]
    out: dict = {}
    i = 0

    def is_str(t: str) -> bool:
        return t[:1] in "\"'"

    while i < len(toks) and toks[i] != "}":
        key = toks[i]
        key = ts_string(key) if is_str(key) else key
        if toks[i + 1] != ":":
            raise ValueError(f"{path}: expected ':' after {key}")
        i += 2
        if toks[i] == "{":
            forms = {}
            i += 1
            while toks[i] != "}":
                form, colon, val = toks[i], toks[i + 1], toks[i + 2]
                if colon != ":" or not is_str(val):
                    raise ValueError(f"{path}: bad plural {key}")
                forms[form] = ts_string(val)
                i += 3
                if toks[i] == ",":
                    i += 1
            out[key] = forms
            i += 1
        elif is_str(toks[i]):
            out[key] = ts_string(toks[i])
            i += 1
        else:
            raise ValueError(f"{path}: value of {key} is not a string literal")
        if toks[i] == ",":
            i += 1
    return out


def placeholders(v) -> list[str]:
    texts = v.values() if isinstance(v, dict) else [v]
    return sorted(set(p for t in texts for p in PLACEHOLDER.findall(t)))


def check_desktop(allow_missing: bool) -> None:
    en = load_ts(I18N / "en.ts", "en")
    for path in sorted(I18N.glob("*.ts")):
        lang = path.stem
        if lang in ("en", "index"):
            continue
        try:
            tr = load_ts(path, lang)
        except (ValueError, IndexError) as e:
            errors.append(f"desktop {lang}: cannot read {path}: {e}")
            continue
        for k in sorted(set(tr) - set(en)):
            errors.append(f"desktop {lang}: {k} is not in en.ts")
        for k in en:
            if k not in tr:
                missing(f"desktop {lang}: missing {k} (shows in English)", allow_missing)
                continue
            ev, tv = en[k], tr[k]
            if isinstance(ev, dict) != isinstance(tv, dict):
                errors.append(f"desktop {lang}: {k} plural/plain shape differs from English")
                continue
            if isinstance(ev, dict) and set(ev) != set(tv):
                errors.append(f"desktop {lang}: {k} plural forms {sorted(tv)} != English {sorted(ev)}")
            if placeholders(ev) != placeholders(tv):
                errors.append(f"desktop {lang}: {k} placeholders {placeholders(tv)} != English {placeholders(ev)}")


def check_registered() -> None:
    """Every language with strings is also offered: Android's locales_config.xml
    (Android 13+ per-app language) and the desktop's index.ts (imported, in
    `catalogues`, and in the Settings picker's LANGUAGES)."""
    cfg = RES / "xml/locales_config.xml"
    try:
        locales = {el.get("{http://schemas.android.com/apk/res/android}name") for el in ET.parse(cfg).getroot()}
    except (ET.ParseError, OSError) as e:
        errors.append(f"android: cannot read {cfg}: {e}")
        locales = set()
    for p in sorted(RES.glob("values-*/strings.xml")):
        lang = p.parent.name.removeprefix("values-")
        if locales and lang not in locales:
            errors.append(f"android {lang}: not listed in res/xml/locales_config.xml")
    index = (I18N / "index.ts").read_text(encoding="utf-8")
    for p in sorted(I18N.glob("*.ts")):
        lang = p.stem
        if lang == "index":
            continue
        if not re.search(rf'\{{\s*id:\s*"{lang}"', index):
            errors.append(f"desktop {lang}: not in LANGUAGES (index.ts)")
        if lang != "en" and not re.search(rf'import \{{ {lang} \}} from "\./{lang}"', index):
            errors.append(f"desktop {lang}: not imported in index.ts")


def main() -> int:
    allow = "--allow-missing" in sys.argv[1:]
    check_android(allow)
    check_desktop(allow)
    check_registered()
    for w in warnings:
        print("warning:", w)
    for e in errors:
        print("error:", e)
    langs_a = sorted(p.parent.name.removeprefix("values-") for p in RES.glob("values-*/strings.xml"))
    langs_d = sorted(p.stem for p in I18N.glob("*.ts") if p.stem not in ("en", "index"))
    print(f"checked android {langs_a}, desktop {langs_d}: {len(errors)} error(s), {len(warnings)} warning(s)")
    return 1 if errors else 0


if __name__ == "__main__":
    sys.exit(main())
