"""Wikimedia Commons reference recordings (keyless): Rana temporaria for the frogs_common category.

    uv run python -m asmr_ml.data.refs_commons
"""

import json
import time

from .commons import UA, _download, _strip, list_files  # noqa: F401
from .refs_fetch import REFS

CATS = {"frogs_common": "Rana temporaria"}


def main() -> None:
    mf = REFS / "manifest.jsonl"
    have = {json.loads(l)["id"] for l in mf.read_text().splitlines()} if mf.exists() else set()
    for cat, latin in CATS.items():
        (REFS / cat).mkdir(parents=True, exist_ok=True)
        for pid, title, ii in list_files(latin):
            if not title.lower().endswith(".ogg") or f"commons:{pid}" in have:
                continue
            m = ii["extmetadata"]
            rel = f"{cat}/commons{pid}.ogg"
            (REFS / rel).write_bytes(_download(ii["url"]))
            row = dict(id=f"commons:{pid}", category=cat, path=rel, licence=_strip(m["LicenseShortName"]["value"]),
                       author=_strip(m.get("Artist", {}).get("value", "")),
                       url="https://commons.wikimedia.org/wiki/" + title.replace(" ", "_"), name=title[5:],
                       duration=0, tags=[], rating=None, nratings=None)
            with mf.open("a") as f:
                f.write(json.dumps(row, ensure_ascii=False) + "\n")
            print(cat, row["licence"], title)
            time.sleep(2)


if __name__ == "__main__":
    main()
