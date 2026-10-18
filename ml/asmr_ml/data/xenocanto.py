"""xeno-canto API v3 downloader (needs a free key: https://xeno-canto.org/account).

    XC_API_KEY=... uv run python -m asmr_ml.data.xenocanto chaffinch --quality A --max 100

Only quality A/B songs. NC/ND recordings are written to the manifest with usable=false and are
not downloaded unless --allow-nc is passed (a user decision, SPEC 8.1).
"""

import argparse
import os
import time

import requests

from ..species import SPECIES, licence_ok
from . import manifest
from . import secrets
from .licences import from_url

API = "https://xeno-canto.org/api/3/recordings"


def search(latin: str, key: str, quality: str, kind: str):
    page = 1
    while True:
        q = f'sp:"{latin.lower()}" q:{quality} type:{kind}'
        r = requests.get(API, params={"query": q, "key": key, "page": page}, timeout=30)
        r.raise_for_status()
        d = r.json()
        yield from d.get("recordings", [])
        if page >= int(d.get("numPages", 1)):
            break
        page += 1


def fetch(sp_key: str, key: str, quality: str, kind: str, max_files: int, allow_nc: bool) -> None:
    sp = SPECIES[sp_key]
    have = manifest.load()
    (manifest.RAW / sp_key).mkdir(parents=True, exist_ok=True)
    n = 0
    for rec in search(sp.latin, key, quality, kind):
        if n >= max_files:
            break
        rid = f"xeno-canto:{rec['id']}"
        if rid in have or f"commons:{sp.latin} - {sp.english} XC{rec['id']}.mp3" in have:
            continue  # already have it (possibly via the Commons mirror)
        lic = from_url(rec.get("lic", ""))
        ok = licence_ok(lic)
        path = f"{sp_key}/XC{rec['id']}.mp3"
        m, s = (rec.get("length") or "0:0").split(":")[-2:]
        r = manifest.Record(
            id=rid, source="xeno-canto", species=sp_key, path=path, licence=lic, usable=ok,
            author=rec.get("rec", ""), url=f"https://xeno-canto.org/{rec['id']}",
            kind=rec.get("type", ""), duration=int(m) * 60 + int(s),
            extra={"q": rec.get("q"), "cnt": rec.get("cnt"), "smp": rec.get("smp")},
        )
        if ok or allow_nc:
            url = rec["file"] if rec["file"].startswith("http") else "https:" + rec["file"]
            data = requests.get(url, timeout=120)
            data.raise_for_status()
            (manifest.RAW / path).write_bytes(data.content)
            time.sleep(0.5)
        manifest.append(r)
        n += 1
        print(f"{sp_key}: {lic:14s} {r.kind[:20]:20s} {r.duration:5.0f}s {path}{'' if ok else '  (NC/ND: skipped)'}")


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("species", nargs="*", default=list(SPECIES))
    ap.add_argument("--quality", default="A")
    ap.add_argument("--type", default="song")
    ap.add_argument("--max", type=int, default=100)
    ap.add_argument("--allow-nc", action="store_true", help="download NC/ND too (user decision)")
    a = ap.parse_args()
    key = secrets.get("XC_API_KEY")
    if not key:
        raise SystemExit("set XC_API_KEY (https://xeno-canto.org/account)")
    for s in a.species:
        fetch(s, key, a.quality, a.type, a.max, a.allow_nc)


if __name__ == "__main__":
    main()
