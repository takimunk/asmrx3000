"""Normalise licence URLs from xeno-canto / Freesound to short names ("CC BY-SA 4.0")."""

import re


def from_url(url: str) -> str:
    u = url.lower()
    if "publicdomain/zero" in u or "cc0" in u:
        return "CC0 1.0"
    if "publicdomain/mark" in u:
        return "Public domain"
    m = re.search(r"licenses/([a-z-]+)/([0-9.]+)", u)
    if m:
        return f"CC {m.group(1).upper()} {m.group(2)}"
    return url or "unknown"
