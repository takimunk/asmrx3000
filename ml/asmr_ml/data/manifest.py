"""Licence manifest: one JSON line per recording, kept next to the raw audio (ml/data/raw/).

Every downloader appends here; nothing is trained on unless `usable` is true. The manifest
itself is small and is the attribution list for CC BY / BY-SA material.
"""

import json
from dataclasses import asdict, dataclass, field
from pathlib import Path

DATA = Path(__file__).resolve().parents[2] / "data"
RAW = DATA / "raw"
MANIFEST = RAW / "manifest.jsonl"


@dataclass
class Record:
    id: str  # "<source>:<native id>"
    source: str  # commons | xeno-canto | freesound | elevenlabs
    species: str  # Species.key, or a texture key such as "stream"
    path: str  # relative to RAW
    licence: str  # short form, e.g. "CC BY-SA 4.0"
    usable: bool  # licence acceptable without a user decision
    author: str = ""
    url: str = ""
    kind: str = ""  # song / call / ... when the source says
    duration: float = 0.0
    extra: dict = field(default_factory=dict)


def load() -> dict[str, Record]:
    out: dict[str, Record] = {}
    if MANIFEST.exists():
        for line in MANIFEST.read_text().splitlines():
            if line.strip():
                r = Record(**json.loads(line))
                out[r.id] = r
    return out


def append(r: Record) -> None:
    MANIFEST.parent.mkdir(parents=True, exist_ok=True)
    with MANIFEST.open("a") as f:
        f.write(json.dumps(asdict(r), ensure_ascii=False) + "\n")


def usable(species: str | None = None) -> list[Record]:
    return [r for r in load().values() if r.usable and (species is None or r.species == species)]
