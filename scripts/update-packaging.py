#!/usr/bin/env python3
"""Regenerate the AUR, winget and Scoop manifests for a release.

Usage:
    scripts/update-packaging.py 0.7.0 [--sums SHA256SUMS]

Without --sums the sums are downloaded from the GitHub release with `gh`.
The AUR PKGBUILD is patched in place; the winget and Scoop files are written
under packaging/. Commit the result (and copy the winget folder into a
winget-pkgs fork) after checking `sha256sum -c`.
"""

import argparse
import hashlib
import re
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
            return parse_sums(Path(tmp) / "SHA256SUMS")
    return parse_sums(path)


def parse_sums(path: Path) -> dict[str, str]:
    sums = {}
    for line in path.read_text().splitlines():
        if not line.strip():
            continue
        digest, name = line.split()
        sums[name] = digest
    return sums


def asset(version: str, target: str) -> str:
    return f"touchpad-snap-v{version}-{target}"


def zip_name(version: str, target: str) -> str:
    return asset(version, target) + ".zip"


def linux_name(version: str, target: str) -> str:
    return asset(version, target) + ".tar.gz"


def write(path: Path, text: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text)


def update_pkgbuild(version: str, sums: dict[str, str]) -> None:
    path = ROOT / "packaging/aur/touchpad-snap-bin/PKGBUILD"
    text = path.read_text()
    text = re.sub(r"^pkgver=.*$", f"pkgver={version}", text, flags=re.MULTILINE)
    text = re.sub(r"^pkgrel=.*$", "pkgrel=1", text, flags=re.MULTILINE)
    for arch, target, key in (
        ("x86_64", TARGETS["linux_x64"], "linux_x64"),
        ("aarch64", TARGETS["linux_arm64"], "linux_arm64"),
    ):
        shas = [sums[linux_name(version, target)], sums[UUID + ".shell-extension.zip"]]
        block = (
            f"sha256sums_{arch}=('{shas[0]}'\n"
            f"{' ' * (len(f'sha256sums_{arch}=') + 1)}'{shas[1]}')"
        )
        text, count = re.subn(
            rf"^sha256sums_{arch}=\(.*?\)$", block, text, flags=re.MULTILINE | re.DOTALL
        )
        if count != 1:
            sys.exit(f"could not update sha256sums_{arch} in {path}")
    path.write_text(text)


def update_winget(version: str, sums: dict[str, str]) -> None:
    folder = ROOT / f"packaging/winget/manifests/h/hjosugi/touchpad-snap/{version}"
    write(folder / "hjosugi.touchpad-snap.yaml", f"""PackageIdentifier: hjosugi.touchpad-snap
PackageVersion: {version}
DefaultLocale: en-US
ManifestType: version
ManifestVersion: 1.6.0
""")

    previous = sorted(
        (ROOT / "packaging/winget/manifests/h/hjosugi/touchpad-snap").glob("*/hjosugi.touchpad-snap.locale.en-US.yaml")
    )
    locale = previous[-1].read_text() if previous else ""
    if locale:
        locale = re.sub(r"^PackageVersion:.*$", f"PackageVersion: {version}", locale, flags=re.MULTILINE)
        locale = re.sub(r"v[0-9]+\.[0-9]+\.[0-9]+", f"v{version}", locale)
    else:
        locale = f"""PackageIdentifier: hjosugi.touchpad-snap
PackageVersion: {version}
PackageLocale: en-US
Publisher: hjosugi
PublisherUrl: https://github.com/hjosugi
PublisherSupportUrl: https://github.com/hjosugi/touchpad-snap/issues
Author: hjosugi
PackageName: touchpad-snap
PackageUrl: https://github.com/hjosugi/touchpad-snap
License: MIT
LicenseUrl: https://github.com/hjosugi/touchpad-snap/blob/main/LICENSE
ShortDescription: Bind touchpad gestures and keyboard shortcuts to window and desktop actions
Moniker: touchpad-snap
Tags:
  - gesture
  - hotkey
  - window-management
  - touchpad
ReleaseNotesUrl: https://github.com/hjosugi/touchpad-snap/releases/tag/v{version}
ManifestType: defaultLocale
ManifestVersion: 1.6.0
"""
    write(folder / "hjosugi.touchpad-snap.locale.en-US.yaml", locale)

    installers = []
    for arch, target in (("x64", TARGETS["x64"]), ("arm64", TARGETS["arm64"])):
        installers.append(f"""  - Architecture: {arch}
    InstallerUrl: https://github.com/{REPO}/releases/download/v{version}/{zip_name(version, target)}
    InstallerSha256: {sums[zip_name(version, target)].upper()}
    NestedInstallerFiles:
      - RelativeFilePath: {asset(version, target)}\\touchpad-snap.exe
        PortableCommandAlias: touchpad-snap
      - RelativeFilePath: {asset(version, target)}\\touchpad-snapw.exe
        PortableCommandAlias: touchpad-snapw
""")
    write(folder / "hjosugi.touchpad-snap.installer.yaml", f"""PackageIdentifier: hjosugi.touchpad-snap
PackageVersion: {version}
InstallerType: zip
NestedInstallerType: portable
Installers:
{''.join(installers)}ManifestType: installer
ManifestVersion: 1.6.0
""")


def update_scoop(version: str, sums: dict[str, str]) -> None:
    write(ROOT / "packaging/scoop/touchpad-snap.json", f"""{{
  "version": "{version}",
  "description": "Bind touchpad gestures and keyboard shortcuts to window and desktop actions.",
  "homepage": "https://github.com/{REPO}",
  "license": "MIT",
  "architecture": {{
    "64bit": {{
      "url": "https://github.com/{REPO}/releases/download/v{version}/{zip_name(version, TARGETS['x64'])}",
      "hash": "{sums[zip_name(version, TARGETS['x64'])]}",
      "extract_dir": "{asset(version, TARGETS['x64'])}"
    }},
    "arm64": {{
      "url": "https://github.com/{REPO}/releases/download/v{version}/{zip_name(version, TARGETS['arm64'])}",
      "hash": "{sums[zip_name(version, TARGETS['arm64'])]}",
      "extract_dir": "{asset(version, TARGETS['arm64'])}"
    }}
  }},
  "bin": [
    "touchpad-snap.exe",
    "touchpad-snapw.exe"
  ],
  "checkver": {{
    "github": "https://github.com/{REPO}"
  }},
  "autoupdate": {{
    "architecture": {{
      "64bit": {{
        "url": "https://github.com/{REPO}/releases/download/v$version/touchpad-snap-v$version-x86_64-pc-windows-msvc.zip",
        "extract_dir": "touchpad-snap-v$version-x86_64-pc-windows-msvc"
      }},
      "arm64": {{
        "url": "https://github.com/{REPO}/releases/download/v$version/touchpad-snap-v$version-aarch64-pc-windows-msvc.zip",
        "extract_dir": "touchpad-snap-v$version-aarch64-pc-windows-msvc"
      }}
    }},
    "hash": {{
      "url": "https://github.com/{REPO}/releases/download/v$version/SHA256SUMS"
    }}
  }}
}}
""")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("version")
    parser.add_argument("--sums", type=Path)
    args = parser.parse_args()

    sums = sums_for(args.version, args.sums)
    required = {
        UUID + ".shell-extension.zip",
        *(linux_name(args.version, t) for t in (TARGETS["linux_x64"], TARGETS["linux_arm64"])),
        *(zip_name(args.version, t) for t in (TARGETS["x64"], TARGETS["arm64"])),
    }
    missing = required - sums.keys()
    if missing:
        sys.exit(f"SHA256SUMS is missing: {', '.join(sorted(missing))}")

    update_pkgbuild(args.version, sums)
    update_winget(args.version, sums)
    update_scoop(args.version, sums)
    print(f"Updated packaging for {args.version}")
    for name in sorted(required):
        print(f"  {sums[name]}  {name}")


if __name__ == "__main__":
    main()
