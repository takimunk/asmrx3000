"""Keyless downloader: bird recordings on Wikimedia Commons ("Audio files of <Latin>").

Most are xeno-canto uploads that the recordist released under CC BY / BY-SA, so they are the
commercially safe subset of xeno-canto. Commons only hosts free licences, but we still check
each file's licence and record it in the manifest.

    uv run python -m asmr_ml.data.commons chaffinch robin --max 60
"""

import argparse
import html
import json
import os
import re
import time

import requests
from dataclasses import asdict

from ..species import SPECIES, licence_ok
from . import manifest

API = "https://commons.wikimedia.org/w/api.php"
# Wikimedia's User-Agent policy wants a URL in the UA; without one uploads get 429.
UA = {
    "User-Agent": os.environ.get(
        "ASMR_USER_AGENT",
        "asmrx3000-ml/0.1 (bird song synthesis research; "
        "+https://commons.wikimedia.org/wiki/Commons:Reusing_content_outside_Wikimedia) python-requests",
    )
}


def _strip(s: str) -> str:
    return html.unescape(re.sub(r"<[^>]+>", "", s or "")).strip()


def list_files(latin: str):
    params = {
        "action": "query",
        "format": "json",
        "generator": "categorymembers",
        "gcmtitle": f"Category:Audio files of {latin}",
        "gcmtype": "file",
        "gcmlimit": 50,
        "prop": "imageinfo",
        "iiprop": "url|size|mime|extmetadata",
    }
    while True:
        d = requests.get(API, params=params, headers=UA, timeout=30).json()
        for p in d.get("query", {}).get("pages", {}).values():
            if p.get("imageinfo"):
                yield p["pageid"], p["title"], p["imageinfo"][0]
        if "continue" not in d:
            break
        params.update(d["continue"])


def _download(url: str) -> bytes:
    """upload.wikimedia.org rate-limits bursts (429): back off and retry."""
    for attempt in range(8):
        r = requests.get(url, headers=UA, timeout=120)
        if r.status_code == 429:
            time.sleep(float(r.headers.get("Retry-After", 0)) or 5 * 2**attempt)
            continue
        r.raise_for_status()
        time.sleep(1.0)
        return r.content
    raise RuntimeError(f"still rate-limited: {url}")


def fetch(key: str, max_files: int) -> None:
    sp = SPECIES[key]
    have = manifest.load()
    out_dir = manifest.RAW / key
    out_dir.mkdir(parents=True, exist_ok=True)
    n = 0
    seen_xc = {r.extra.get("xc") for r in have.values() if r.extra.get("xc")}
    for pageid, title, info in list_files(sp.latin):
        if n >= max_files:
            break
        rid = "commons:" + title.removeprefix("File:")
        if rid in have:
            n += 1
            continue
        meta = info.get("extmetadata", {})
        lic = _strip(meta.get("LicenseShortName", {}).get("value", "unknown"))
        desc = _strip(meta.get("ImageDescription", {}).get("value", ""))
        kind = ""
        m = re.search(r"Type:\s*([^\n]+)", desc)
        if m:
            kind = m.group(1).strip()
        ext = title.rsplit(".", 1)[-1].lower()
        # Commons often has the same xeno-canto recording twice (mp3 + ogg): keep one.
        xc = re.search(r"(?:XC|xeno-canto\.org/)(\d+)", title + " " + _strip(meta.get("Credit", {}).get("value", "")))
        xc = xc.group(1) if xc else None
        if xc and xc in seen_xc:
            continue
        seen_xc.add(xc)
        fname = f"{pageid}_" + re.sub(r"[^A-Za-z0-9_.-]+", "_", title.removeprefix("File:"))[-80:]
        rec = manifest.Record(
            id=rid,
            source="commons",
            species=key,
            path=f"{key}/{fname}",
            licence=lic,
            usable=licence_ok(lic),
            author=_strip(meta.get("Artist", {}).get("value", "")),
            url=info.get("descriptionurl", ""),
            kind=kind,
            duration=float(info.get("duration") or 0.0),
            extra={"mime": info.get("mime", ""), "ext": ext, "xc": xc},
        )
        if rec.usable:
            (manifest.RAW / rec.path).write_bytes(_download(info["url"]))
        manifest.append(rec)
        n += 1
        print(f"{key}: {rec.licence:14s} {rec.kind[:24]:24s} {rec.duration:6.1f}s {fname}")


def refresh_meta() -> None:
    """Backfill Commons categories into the manifest (to drop multi-species soundscapes)."""
    recs = manifest.load()
    commons = [r for r in recs.values() if r.source == "commons"]
    for i in range(0, len(commons), 50):
        chunk = commons[i:i + 50]
        titles = "|".join("File:" + r.id.removeprefix("commons:") for r in chunk)
        d = requests.post(API, data={"action": "query", "format": "json", "titles": titles, "prop": "categories",
                                     "cllimit": "max"}, headers=UA, timeout=30).json()
        norm = {n["to"]: n["from"] for n in d["query"].get("normalized", [])}
        for p in d["query"]["pages"].values():
            title = norm.get(p["title"], p["title"])
            cats = [c["title"].removeprefix("Category:") for c in p.get("categories", [])]
            r = recs.get("commons:" + title.removeprefix("File:"))
            if r:
                r.extra["categories"] = cats
                r.extra["species_in_file"] = sum(c.startswith("Audio files of ") for c in cats)
    manifest.MANIFEST.write_text("".join(json.dumps(asdict(r), ensure_ascii=False) + "\n" for r in recs.values()))
    multi = [r.id for r in commons if r.extra.get("species_in_file", 1) > 1]
    print(f"{len(commons)} commons records, {len(multi)} with more than one species")


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("species", nargs="*", default=list(SPECIES))
    ap.add_argument("--max", type=int, default=200)
    ap.add_argument("--refresh-meta", action="store_true", help="only backfill categories")
    a = ap.parse_args()
    if a.refresh_meta:
        refresh_meta()
        return
    for key in a.species:
        fetch(key, a.max)


if __name__ == "__main__":
    main()
