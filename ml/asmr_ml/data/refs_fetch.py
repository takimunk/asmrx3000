"""Fetch reference field recordings per category into ml/data/refs/<category>/ (CC0 / CC BY only).

    uv run python -m asmr_ml.data.refs_fetch [category ...] [--max 8]

Freesound (token auth, HQ ogg previews). Writes ml/data/refs/manifest.jsonl with licence,
author, URL for attribution. Skips names/tags that suggest music, voices or sfx packs.
"""

import argparse
import json
import re
import time

import requests

from ..species import licence_ok
from . import secrets
from .freesound import API, QUERIES
from .licences import from_url
from .manifest import DATA

REFS = DATA / "refs"
# Freesound ANDs all words, so several short queries per category (tried in order).
REF_QUERIES = {
    "thunder_close": ["thunder crack", "thunder clap", "lightning strike", "thunder close"],
    "thunder_distant": ["distant thunder", "thunder rumble", "thunder rolling"],
    "lake_lapping": ["lake waves", "lapping water", "lake shore", "waves pebbles", "water lapping"],
    "still_lake_ambience": ["lake ambience", "lake ambient", "pond ambience", "calm lake"],
    "brook": ["brook", "stream", "creek", "babbling", "babbling brook"],
    "slow_river": ["river", "slow river", "river flowing", "river ambience"],
    "waterfall_small": ["small waterfall", "waterfall", "cascade"],
    "wind_trees": ["wind trees", "wind forest", "wind leaves", "wind in trees"],
    "wind_open": ["wind grass", "wind field", "wind meadow", "wind open"],
    "wind_gusty": ["wind gusts", "gusty wind", "wind gust"],
    "wings_pigeon": ["pigeon wings", "pigeon take off", "pigeon flapping", "dove wings", "wood pigeon"],
    "wings_small": ["sparrow take off", "starling flock take off", "birds flush wings", "bird flutter", "bird wings", "bird take off", "sparrow wings", "bird flapping"],
    "wings_large": ["swan wings", "swan flying", "geese take off", "goose wings", "heron wings", "seagull wings", "crow wings", "swan wings", "wing flap", "wingbeats", "goose wings"],
    "rain_forest": ["rain forest", "rain leaves", "rain in forest", "rain woods"],
}
REF_QUERIES["frogs_common"] = ["common frog", "rana temporaria", "brown frog", "grass frog", "frog spawning chorus", "frogs pond", "frog pond spring", "frogs spawning"]
MIN_DUR = {"thunder_close": 4, "wings_pigeon": 2, "wings_small": 2, "wings_large": 3}
MAX_DUR = 200
BAD = re.compile(
    r"music|song|voice|speech|talk|synth|loop|drone|sfx pack|game|cinematic|horror|ambient pad|"
    r"guitar|piano|human|people|crowd|traffic|car |engine|motor|airplane|aircraft|plane|train|"
    r"shower|toilet|bath|tap |faucet|fountain|pool|sink|dishwasher|fan |vacuum|cartoon|"
    r"fire ?works|explosion|gun|cannon|bomb|stormtrooper|fake|synthes|designed|processed|"
    r"granular|reverse|effect", re.I)
ANIMAL_BAD = re.compile(r"city|urban|bell|bird|cricket|cicada|frog|toad|duck|swan|gull|insect|grasshopper|traffic|street|"
                        r"market|night|dawn|chorus|owl|crow|rooster|dog|cow|sheep|church|town|village|garden|boat|ship|bridge|cafe", re.I)
NO_ANIMAL = {"thunder_close", "thunder_distant", "lake_lapping", "still_lake_ambience", "brook", "slow_river",
             "waterfall_small", "wind_trees", "wind_open", "wind_gusty", "rain_forest"}
EXTRA = {
    "lake_lapping": r"lake|wave|lapping|shore|water",
    "still_lake_ambience": r"lake|pond|water|calm",
    "brook": r"brook|stream|creek|babbl|water|burn",
    "slow_river": r"river|stream|water|flow",
    "waterfall_small": r"waterfall|cascade|falls",
    "wind_trees": r"wind|breeze|gust",
    "wind_open": r"wind|breeze|gust",
    "wind_gusty": r"wind|gust",
    "frogs_common": r"frog|rana",  # category -> must-match regex on name+tags (keeps obvious mismatches out)
    "thunder_close": r"thunder|lightning|crack|strike",
    "thunder_distant": r"thunder",
    "wings_pigeon": r"pigeon|dove|columba|wing|flap|duiven",
    "wings_small": r"wing|flap|flutter|bird|take.?off",
    "wings_large": r"wing|flap|gull|crow|swan|goose|geese|raven|heron|duck",
    "rain_forest": r"rain",
}


def fetch(cat: str, token: str, n_max: int) -> list[dict]:
    d_out = REFS / cat
    d_out.mkdir(parents=True, exist_ok=True)
    have = {json.loads(l)["id"] for l in (REFS / "manifest.jsonl").read_text().splitlines()} if (REFS / "manifest.jsonl").exists() else set()
    lo = MIN_DUR.get(cat, 20)
    n_max -= sum(1 for l in (REFS / "manifest.jsonl").read_text().splitlines() if json.loads(l)["category"] == cat) if (REFS / "manifest.jsonl").exists() else 0
    got = []
    for q in REF_QUERIES.get(cat, [QUERIES[cat]]):
        if len(got) < n_max:
            got += fetch_q(cat, q, token, n_max - len(got), have | {r["id"] for r in got})
    return got


def fetch_q(cat: str, query: str, token: str, n_max: int, have: set) -> list[dict]:
    d_out = REFS / cat
    lo = MIN_DUR.get(cat, 20)
    params = {
        "query": query,
        "filter": f'license:("Creative Commons 0" OR "Attribution") duration:[{lo} TO {MAX_DUR}]',
        "fields": "id,name,license,username,previews,duration,url,tags,avg_rating,num_ratings,description",
        "page_size": 60, "sort": "rating_desc", "token": token,
    }
    got, page = [], 0
    url = API
    while url and len(got) < n_max and page < 3:
        d = requests.get(url, params=params, timeout=30).json()
        params = {"token": token}
        page += 1
        for s in d.get("results", []):
            if len(got) >= n_max:
                break
            text = s["name"] + " " + " ".join(s["tags"])
            if BAD.search(s["name"]) or BAD.search(" ".join(s["tags"])):
                continue
            if cat in NO_ANIMAL and ANIMAL_BAD.search(text):
                continue
            if cat in EXTRA and not re.search(EXTRA[cat], text, re.I):
                continue
            lic = from_url(s["license"])
            if not licence_ok(lic):
                continue
            rid = f"freesound:{s['id']}"
            rel = f"{cat}/fs{s['id']}.ogg"
            if rid not in have:
                try:
                    a = requests.get(s["previews"]["preview-hq-ogg"], params={"token": token}, timeout=120)
                    a.raise_for_status()
                except requests.RequestException as e:
                    print("download failed, skipping:", type(e).__name__, flush=True)
                    continue
                (REFS / rel).write_bytes(a.content)
                time.sleep(0.3)
                rec = dict(id=rid, category=cat, path=rel, licence=lic, author=s["username"], url=s["url"],
                           name=s["name"], duration=float(s["duration"]), tags=s["tags"][:12],
                           rating=s.get("avg_rating"), nratings=s.get("num_ratings"))
                with (REFS / "manifest.jsonl").open("a") as f:
                    f.write(json.dumps(rec, ensure_ascii=False) + "\n")
                got.append(rec)
                print(f"{cat}: {lic:10s} {rec['duration']:6.1f}s r={rec['rating'] or 0:.1f} {s['name'][:60]}", flush=True)
        url = d.get("next")
    return got


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("cats", nargs="*")
    ap.add_argument("--max", type=int, default=8)
    a = ap.parse_args()
    token = secrets.get("FREESOUND_API_KEY")
    if not token:
        raise SystemExit("set FREESOUND_API_KEY")
    if len(token) > 40:  # Freesound keys are 40 chars; ml/.env has stray trailing characters
        token = token[:40]
    cats = a.cats or [c for c in QUERIES if c not in ("stream", "river", "leaves")]
    for c in cats:
        fetch(c, token, a.max)


if __name__ == "__main__":
    main()
