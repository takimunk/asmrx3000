#!/usr/bin/env python3
"""Keeps the macOS app's strings honest.

 - every key used in the Swift sources, tr("…"), trn("…") (plural) or N_("…"), has a Russian
   translation in apps/macos/Resources/Localization/ru.lproj;
 - every Russian entry is used (no dead strings) and has the same printf placeholders as its key;
 - plurals have all four Russian forms and the English one/other forms;
 - no Cyrillic outside comments in the Swift sources (a line may opt out with `l10n-ok` in a comment:
   the bilingual language title; a file with `l10n-ok-file` in it is exempt: the LLM prompts and the offline parser's Russian keywords).
Exit status 1 on any problem."""
import json, pathlib, re, subprocess, sys

root = pathlib.Path(__file__).resolve().parent.parent
src = root / "apps/macos/Sources/ASMRX3000"
loc = root / "apps/macos/Resources/Localization"

def strings(path):
    out = {}
    for f in sorted(path.glob("*.strings")):
        data = json.loads(subprocess.check_output(["plutil", "-convert", "json", "-o", "-", str(f)]))
        for k, v in data.items():
            if k in out:
                print(f"duplicate key across files: {k!r} ({f.name})"); globals()["bad"] = True
            out[k] = v
    return out

bad = False
ru = strings(loc / "ru.lproj")
en = strings(loc / "en.lproj") if (loc / "en.lproj").exists() else {}

lit = r'"((?:[^"\\]|\\.)*)"'
call = re.compile(r'\b(tr|trn|N_)\(\s*' + lit)
used, plural = set(), set()
cyr = re.compile(r'[А-Яа-яЁё]')
for f in sorted(src.rglob("*.swift")):
    text = f.read_text()
    exempt = "l10n-ok-file" in text
    for n, line in enumerate(text.splitlines(), 1):
        code = re.sub(r'(?<!:)//.*$', '', line)
        if cyr.search(code) and "l10n-ok" not in line and not exempt:
            print(f"cyrillic literal: {f.relative_to(root)}:{n}: {line.strip()[:100]}"); bad = True
        for kind, key in call.findall(code):
            key = key.encode().decode("unicode_escape") if "\\" in key else key
            # "\(x)" interpolation inside a key is a bug: keys are constants
            if "\\(" in key or "(x)" in key:
                print(f"interpolated key: {f.relative_to(root)}:{n}: {key}"); bad = True
            (plural if kind == "trn" else used).add(key)

def ph(s): return sorted(re.findall(r'%(?:\d\$)?[-0-9.]*(?:ld|lld|d|@|f|s)', s))

for k in sorted(used):
    if k not in ru:
        print(f"missing ru: {k!r}"); bad = True
    elif ph(k) != ph(ru[k]):
        print(f"placeholder mismatch: {k!r} -> {ru[k]!r}"); bad = True
for k in sorted(plural):
    for form in ("one", "few", "many", "other"):
        if f"{k}#{form}" not in ru:
            print(f"missing ru plural: {k}#{form}"); bad = True
    for form in ("one", "other"):
        if f"{k}#{form}" not in en:
            print(f"missing en plural: {k}#{form}"); bad = True
known = used | {f"{k}#{f}" for k in plural for f in ("one", "few", "many", "other")}
for k in sorted(ru):
    if k not in known:
        print(f"unused ru: {k!r}"); bad = True
print(f"{len(used)} strings, {len(plural)} plurals, {len(ru)} ru entries")
sys.exit(1 if bad else 0)
