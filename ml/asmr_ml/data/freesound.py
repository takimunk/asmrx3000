"""Freesound API v2 downloader for textures (stream, river, wind in trees, leaves).

    FREESOUND_API_KEY=... uv run python -m asmr_ml.data.freesound stream --max 40

Uses token auth, which gives the high-quality previews (ogg ~192 kbps), not the originals
(those need OAuth2). Only CC0 / CC BY are requested.
"""

import argparse
import os
import time

import requests

from ..species import licence_ok
from . import manifest
from . import secrets
from .licences import from_url

API = "https://freesound.org/apiv2/search/text/"

QUERIES = {
    "stream": "mountain stream brook water flowing",
    "river": "river flowing water",
    "wind_trees": "wind trees forest leaves",
    "leaves": "leaves rustling",
}


def fetch(texture: str, token: str, max_files: int) -> None:
    have = manifest.load()
    (manifest.RAW / texture).mkdir(parents=True, exist_ok=True)
    params = {
        "query": QUERIES.get(texture, texture),
        "filter": 'license:("Creative Commons 0" OR "Attribution") duration:[20 TO 600]',
        "fields": "id,name,license,username,previews,duration,url,tags",
        "page_size": 50,
        "sort": "rating_desc",
        "token": token,
    }
    url, n = API, 0
    while url and n < max_files:
        d = requests.get(url, params=params, timeout=30).json()
        params = {"token": token}  # `next` already carries the query
        for s in d.get("results", []):
            if n >= max_files:
                break
            rid = f"freesound:{s['id']}"
            if rid in have:
                continue
            lic = from_url(s["license"])
            path = f"{texture}/fs{s['id']}.ogg"
            r = manifest.Record(
                id=rid, source="freesound", species=texture, path=path, licence=lic,
                usable=licence_ok(lic), author=s["username"], url=s["url"],
                kind=texture, duration=float(s["duration"]), extra={"name": s["name"], "tags": s["tags"][:12]},
            )
            if r.usable:
                audio = requests.get(s["previews"]["preview-hq-ogg"], params={"token": token}, timeout=120)
                audio.raise_for_status()
                (manifest.RAW / path).write_bytes(audio.content)
                time.sleep(0.3)
            manifest.append(r)
            n += 1
            print(f"{texture}: {lic:12s} {r.duration:6.1f}s {s['name'][:50]}")
        url = d.get("next")


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("textures", nargs="*", default=["stream"])
    ap.add_argument("--max", type=int, default=40)
    a = ap.parse_args()
    token = secrets.get("FREESOUND_API_KEY")
    if not token:
        raise SystemExit("set FREESOUND_API_KEY (https://freesound.org/apiv2/apply)")
    for t in a.textures:
        fetch(t, token, a.max)


if __name__ == "__main__":
    main()
