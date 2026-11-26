#!/usr/bin/env python3
"""Check the GNOME extension package before uploading it to EGO.

Usage:
    scripts/check-ego.py [version]

Verifies metadata, shell versions, CHANGELOG entry, zip layout (when built)
and screenshot presence. Exits non-zero on a problem; screenshots are a
warning until they are captured on a real session.
"""

import json
import sys
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
UUID = "touchpad-snap@hjosugi.github.io"
EXTENSION = ROOT / "gnome-extension" / UUID
SHELL_VERSIONS = ["46", "48", "49", "50", "51"]


def fail(message: str) -> None:
    print(f"error: {message}", file=sys.stderr)
    raise SystemExit(1)


def warn(message: str) -> None:
    print(f"warning: {message}", file=sys.stderr)


def main() -> None:
    version = sys.argv[1] if len(sys.argv) > 1 else None
    metadata = json.loads((EXTENSION / "metadata.json").read_text())
    if metadata.get("uuid") != UUID:
        fail(f"uuid is {metadata.get('uuid')!r}, expected {UUID!r}")
    for field in ("name", "description", "url", "version-name"):
        if not metadata.get(field):
            fail(f"metadata.json has no {field}")
    if metadata.get("shell-version") != SHELL_VERSIONS:
        fail(
            "shell-version is "
            f"{metadata.get('shell-version')}, expected {SHELL_VERSIONS}"
        )
    if version and metadata.get("version-name") != version:
        fail(
            f"version-name is {metadata.get('version-name')!r}, expected {version!r}"
        )
    if "touchpad-snap" not in metadata["description"]:
        fail("the description must name the touchpad-snap companion daemon")

    if version:
        changelog = (ROOT / "CHANGELOG.md").read_text()
        if f"## [{version}]" not in changelog:
            fail(f"CHANGELOG.md has no ## [{version}] section")
        for name in ("README.md", "README.ja.md"):
            if "protocol 9" not in (ROOT / name).read_text():
                warn(f"{name} does not mention the current extension protocol")

    for name in ("extension.js", "lib.js", "stylesheet.css"):
        if not (EXTENSION / name).is_file():
            fail(f"missing {EXTENSION / name}")

    zip_path = ROOT / "dist" / f"{UUID}.shell-extension.zip"
    if zip_path.is_file():
        with zipfile.ZipFile(zip_path) as archive:
            names = archive.namelist()
            if "metadata.json" not in names:
                fail("the zip does not have metadata.json at its root")
            if not any(name.startswith("extension.js") for name in names):
                fail("the zip does not contain extension.js")
            packed = json.loads(archive.read("metadata.json"))
            if packed.get("version-name") != metadata.get("version-name"):
                fail("the zip's metadata.json is stale; rebuild it")
        print(f"zip: {zip_path} ({len(names)} entries), metadata at root")
    else:
        warn("dist/*.shell-extension.zip is not built (run `task extension-zip`)")

    screenshots = sorted(
        (ROOT / "gnome-extension" / "screenshots").glob("*.png")
    )
    if screenshots:
        print("screenshots: " + ", ".join(p.name for p in screenshots))
    else:
        warn(
            "no screenshots yet; capture at least one on a real GNOME session "
            "(see gnome-extension/PUBLISHING.md)"
        )

    print(f"extension metadata ok: {metadata['name']} {metadata['version-name']}")


if __name__ == "__main__":
    main()
