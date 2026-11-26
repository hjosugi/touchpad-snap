#!/usr/bin/env python3
"""Check the AUR, winget and Scoop manifests against a release's checksums.

Usage:
    scripts/check-packaging.py 0.7.0 [--sums SHA256SUMS]

Run before copying anything to AUR/winget-pkgs/a Scoop bucket. It verifies
versions, URLs and hashes, and that the AUR .SRCINFO agrees with its PKGBUILD.
"""

import argparse
import json
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
REPO = "hjosugi/touchpad-snap"
UUID = "touchpad-snap@hjosugi.github.io"
TARGETS = {
    "x64": "x86_64-pc-windows-msvc",
    "arm64": "aarch64-pc-windows-msvc",
    "linux_x64": "x86_64-unknown-linux-musl",
    "linux_arm64": "aarch64-unknown-linux-musl",
}


def sums_for(version: str, path: Path | None) -> dict[str, str]:
    if path is None:
        with tempfile.TemporaryDirectory() as tmp:
            subprocess.run(
                ["gh", "release", "download", f"v{version}", "--repo", REPO,
                 "--pattern", "SHA256SUMS", "--dir", tmp, "--clobber"],
                check=True,
            )
            path = Path(tmp) / "SHA256SUMS"
    sums = {}
    for line in path.read_text().splitlines():
        if line.strip():
            digest, name = line.split()
            sums[name] = digest
    return sums


def fail(message: str) -> None:
    print(f"error: {message}", file=sys.stderr)
    raise SystemExit(1)


def check(version: str) -> None:
    sums = sums_for(version, args.sums)
    base = f"https://github.com/{REPO}/releases/download/v{version}"

    # Every asset the packages refer to exists in the release.
    needed = {
        f"touchpad-snap-v{version}-{TARGETS['x64']}.zip",
        f"touchpad-snap-v{version}-{TARGETS['arm64']}.zip",
        f"touchpad-snap-v{version}-{TARGETS['linux_x64']}.tar.gz",
        f"touchpad-snap-v{version}-{TARGETS['linux_arm64']}.tar.gz",
        UUID + ".shell-extension.zip",
    }
    missing = needed - sums.keys()
    if missing:
        fail("release SHA256SUMS is missing: " + ", ".join(sorted(missing)))

    # winget
    folder = ROOT / f"packaging/winget/manifests/h/hjosugi/touchpad-snap/{version}"
    for name in (
        "hjosugi.touchpad-snap.yaml",
        "hjosugi.touchpad-snap.locale.en-US.yaml",
        "hjosugi.touchpad-snap.installer.yaml",
    ):
        if not (folder / name).is_file():
            fail(f"missing winget manifest {folder / name}")
    installer = (folder / "hjosugi.touchpad-snap.installer.yaml").read_text()
    for arch, target in (("x64", TARGETS["x64"]), ("arm64", TARGETS["arm64"])):
        zip_name = f"touchpad-snap-v{version}-{target}.zip"
        url = f"{base}/{zip_name}"
        if url not in installer:
            fail(f"winget {arch} installer URL missing")
        if sums[zip_name].upper() not in installer:
            fail(f"winget {arch} hash does not match SHA256SUMS")
        if f"touchpad-snap-v{version}-{target}\\touchpad-snap.exe" not in installer:
            fail(f"winget {arch} does not point at touchpad-snap.exe")

    # Scoop (bucket layout: bucket/touchpad-snap.json)
    scoop_path = ROOT / "packaging/scoop/bucket/touchpad-snap.json"
    scoop = json.loads(scoop_path.read_text())
    if scoop["version"] != version:
        fail(f"scoop version is {scoop['version']}, expected {version}")
    for arch, target in (("64bit", TARGETS["x64"]), ("arm64", TARGETS["arm64"])):
        entry = scoop["architecture"][arch]
        zip_name = f"touchpad-snap-v{version}-{target}.zip"
        if entry["url"] != f"{base}/{zip_name}":
            fail(f"scoop {arch} URL is wrong")
        if entry["hash"] != sums[zip_name]:
            fail(f"scoop {arch} hash does not match SHA256SUMS")
        if entry["extract_dir"] != f"touchpad-snap-v{version}-{target}":
            fail(f"scoop {arch} extract_dir is wrong")

    # AUR
    pkgbuild = (ROOT / "packaging/aur/touchpad-snap-bin/PKGBUILD").read_text()
    srcinfo = (ROOT / "packaging/aur/touchpad-snap-bin/.SRCINFO").read_text()
    aur_assets = [
        f"touchpad-snap-v{version}-{TARGETS['linux_x64']}.tar.gz",
        f"touchpad-snap-v{version}-{TARGETS['linux_arm64']}.tar.gz",
        UUID + ".shell-extension.zip",
    ]
    aur_sums = [sums[name] for name in aur_assets]
    for text, label in ((pkgbuild, "PKGBUILD"), (srcinfo, ".SRCINFO")):
        if f"pkgver={version}" not in text and f"pkgver = {version}" not in text:
            fail(f"AUR {label} pkgver is not {version}")
        for name in aur_assets:
            # PKGBUILD builds the names from ${pkgver}; .SRCINFO is literal.
            needle = name if label == ".SRCINFO" else name.replace(version, "${pkgver}")
            if needle not in text:
                fail(f"AUR {label} does not reference {name}")
        for digest in aur_sums:
            if digest not in text:
                fail(f"AUR {label} does not contain hash {digest}")
    if "pkgname = touchpad-snap-bin" not in srcinfo:
        fail("AUR .SRCINFO has no pkgname")

    print(f"packaging for {version} is consistent")
    print(f"  winget: {folder}")
    print(f"  AUR:    packaging/aur/touchpad-snap-bin (PKGBUILD + .SRCINFO)")
    print(f"  Scoop:  {scoop_path}")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("version")
    parser.add_argument("--sums", type=Path)
    args = parser.parse_args()
    check(args.version)
