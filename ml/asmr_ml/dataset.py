"""The preparation manifest is the source of truth for accepted training recordings."""

import json
import os
from pathlib import Path


def prepared_files(directory: Path) -> list[Path]:
    report = directory / "prep.json"
    if not report.exists():
        raise FileNotFoundError(f"run preparation first: {report} is missing")
    files = []
    for record in json.loads(report.read_text()):
        if "file" not in record:
            continue
        name = record["file"]
        if Path(name).name != name or not name.endswith(".npz"):
            raise ValueError(f"invalid prepared recording path: {name!r}")
        path = directory / name
        if not path.is_file():
            raise FileNotFoundError(f"prepared recording is missing: {path}")
        files.append(path)
    return sorted(files)


def publish_prepared(staging: Path, directory: Path) -> None:
    """Publish unique recording filenames first, then atomically commit their manifest.

    An interrupted publication leaves the previous manifest and recordings readable.
    The caller supplies generation-prefixed filenames so existing data is never overwritten.
    """
    files = prepared_files(staging)
    if any((directory / path.name).exists() for path in files):
        raise FileExistsError("preparation must use new recording filenames")
    for path in files:
        os.replace(path, directory / path.name)
    os.replace(staging / "prep.json", directory / "prep.json")
    accepted = {path.name for path in files}
    for path in directory.glob("*.npz"):
        if path.name not in accepted:
            path.unlink()
