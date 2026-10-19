"""Reference recordings from xeno-canto (frogs, woodpecker drumming) into ml/data/refs/<cat>/.

    uv run python -m asmr_ml.data.refs_xc [category ...] [--max 8]

Same manifest as refs_fetch (ml/data/refs/manifest.jsonl). Only CC0 / CC BY(-SA) accepted.
"""

import argparse
import json
import time

import requests

from ..species import licence_ok
from . import secrets
from .licences import from_url
from .refs_fetch import REFS

API = "https://xeno-canto.org/api/3/recordings"
CATS = {
    "frogs_marsh": ('sp:"Pelophylax ridibundus"', ""),
    "frogs_tree": ('sp:"Hyla arborea"', ""),
    "frogs_common": ('sp:"Rana temporaria"', ""),
    "woodpecker_drum": ('sp:"Dendrocopos major"', "type:drumming"),
}


def secs(s: str) -> float:
    p = [float(x) for x in (s or "0:0").split(":")]
    return sum(v * 60 ** i for i, v in enumerate(reversed(p)))


def fetch(cat: str, key: str, n_max: int) -> None:
    for qual in ("A", "B", "C", "D", "E"):
        fetch_q(cat, key, n_max, qual)


def fetch_q(cat: str, key: str, n_max: int, qual: str) -> None:
    q, extra = CATS[cat]
    (REFS / cat).mkdir(parents=True, exist_ok=True)
    mf = REFS / "manifest.jsonl"
    have = {json.loads(l)["id"] for l in mf.read_text().splitlines()} if mf.exists() else set()
    got = sum(1 for l in mf.read_text().splitlines() if json.loads(l)["category"] == cat) if mf.exists() else 0
    page = 1
    while got < n_max:
        r = requests.get(API, params={"query": f"{q} {extra} q:{qual}".strip(), "key": key, "page": page}, timeout=30)
        r.raise_for_status()
        d = r.json()
        recs = d.get("recordings", [])
        for rec in recs:
            if got >= n_max:
                break
            lic = from_url(rec.get("lic", ""))
            if not licence_ok(lic) or secs(rec.get("length")) < 8:
                continue
            rid = f"xeno-canto:{rec['id']}"
            if rid in have:
                continue
            url = rec["file"] if rec["file"].startswith("http") else "https:" + rec["file"]
            a = requests.get(url, timeout=120)
            a.raise_for_status()
            rel = f"{cat}/XC{rec['id']}.mp3"
            (REFS / rel).write_bytes(a.content)
            row = dict(id=rid, category=cat, path=rel, licence=lic, author=rec.get("rec", ""),
                       url=f"https://xeno-canto.org/{rec['id']}", name=f"{rec.get('en','')} ({rec.get('gen','')} {rec.get('sp','')}) {rec.get('type','')}",
                       duration=secs(rec.get("length")), tags=[rec.get("type", ""), rec.get("cnt", "")],
                       rating=None, nratings=None)
            with mf.open("a") as f:
                f.write(json.dumps(row, ensure_ascii=False) + "\n")
            got += 1
            print(f"{cat}: {lic:10s} {row['duration']:6.1f}s {row['name'][:60]}", flush=True)
            time.sleep(0.5)
        if page >= int(d.get("numPages", 1)):
            break
        page += 1


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("cats", nargs="*", default=list(CATS))
    ap.add_argument("--max", type=int, default=8)
    a = ap.parse_args()
    key = secrets.get("XC_API_KEY")
    if not key:
        raise SystemExit("set XC_API_KEY")
    for c in a.cats:
        fetch(c, key, a.max)


if __name__ == "__main__":
    main()
