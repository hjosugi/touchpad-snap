#!/usr/bin/env python3
"""Build the extensions.gnome.org zip with metadata.json at its root.

Usage:
    scripts/extension-zip.py

Writes dist/touchpad-snap@hjosugi.github.io.shell-extension.zip. Uses only
Python's zipfile so it works on machines without the `zip` tool, and the
release workflow uses the same script.
"""

import sys
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
UUID = "touchpad-snap@hjosugi.github.io"
SOURCE = ROOT / "gnome-extension" / UUID
OUT = ROOT / "dist" / f"{UUID}.shell-extension.zip"


def main() -> None:
    if not (SOURCE / "metadata.json").is_file():
        sys.exit(f"error: {SOURCE}/metadata.json is missing")
    OUT.parent.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(OUT, "w", zipfile.ZIP_DEFLATED) as archive:
        for path in sorted(SOURCE.rglob("*")):
            if path.is_file():
                archive.write(path, path.relative_to(SOURCE).as_posix())
    print(f"wrote {OUT} ({OUT.stat().st_size} bytes)")


if __name__ == "__main__":
    main()
