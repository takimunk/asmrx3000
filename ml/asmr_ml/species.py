"""Species we model, with the band their song lives in (used for filtering and f0 search)."""

from dataclasses import dataclass


@dataclass(frozen=True)
class Species:
    key: str  # short id used in file names and in the Rust weight files
    latin: str
    english: str
    f0_min: float  # Hz, f0 search range for the song
    f0_max: float


SPECIES = {
    s.key: s
    for s in [
        Species("chaffinch", "Fringilla coelebs", "Common Chaffinch", 1500, 8000),
        Species("song_thrush", "Turdus philomelos", "Song Thrush", 1200, 8000),
        Species("robin", "Erithacus rubecula", "European Robin", 1800, 9000),
        Species("nightingale", "Luscinia megarhynchos", "Common Nightingale", 1000, 9000),
        Species("blackbird", "Turdus merula", "Common Blackbird", 1000, 6000),
    ]
}

# Licences we accept without asking. NC / ND are recorded in the manifest but skipped
# until the user decides (SPEC 8.1).
OPEN_LICENCES = ("cc0", "cc-by", "cc-by-sa", "public domain", "pd")


def licence_ok(short: str) -> bool:
    s = short.lower().replace(" ", "-")
    if "nc" in s.split("-") or "nd" in s.split("-"):
        return False
    return s.startswith(("cc0", "cc-by", "public-domain", "pd")) or s in OPEN_LICENCES
