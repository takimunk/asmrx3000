"""API keys: environment first, then ml/.env (gitignored, KEY=value lines)."""

import os
from pathlib import Path

ENV_FILE = Path(__file__).resolve().parents[2] / ".env"


def get(name: str) -> str | None:
    if os.environ.get(name):
        return os.environ[name]
    if ENV_FILE.exists():
        for line in ENV_FILE.read_text().splitlines():
            k, _, v = line.partition("=")
            if k.strip() == name and v.strip():
                return v.strip().strip("'\"")
    return None
