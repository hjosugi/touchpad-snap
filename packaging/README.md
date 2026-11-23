# Packages

Prebuilt packages other than the GitHub release archives. Everything here is
generated for a release by `scripts/update-packaging.py <version>`, which reads
the published `SHA256SUMS` (`gh release download v<version> --pattern
SHA256SUMS`). The release workflow runs the same script after publishing and
uploads the result as the `packaging-v<version>` artifact.

## AUR

`aur/touchpad-snap-bin/PKGBUILD` is the binary package: it unpacks the musl
archives for x86_64 and aarch64, installs the systemd user unit and the GNOME
extension under `/usr/share/gnome-shell/extensions`, and provides
`touchpad-snap`.

To publish a new version:

1. Download the `packaging-v<version>` artifact, or run
   `scripts/update-packaging.py <version>`.
2. `cd packaging/aur/touchpad-snap-bin && makepkg --printsrcinfo > .SRCINFO`
   (needs `unzip`).
3. Add the `PKGBUILD` and `.SRCINFO` to the AUR git repository
   (`ssh://aur@aur.archlinux.org/touchpad-snap-bin.git`) and push.

The extension is also updated by hand on extensions.gnome.org
(`gnome-extension/PUBLISHING.md`); the AUR package ships the same zip.

## winget

`winget/manifests/h/hjosugi/touchpad-snap/<version>/` holds the three
manifests. To publish, copy that folder unchanged into a fork of
`microsoft/winget-pkgs` under the same path and open a PR. The installer is
the portable zip: winget installs `touchpad-snap.exe` as `touchpad-snap` and
the console-less `touchpad-snapw.exe` as `touchpad-snapw`.

Validate locally with `winget validate --manifest <folder>` on Windows, or let
the winget-pkgs CI do it.

## Scoop

`scoop/touchpad-snap.json` is the bucket entry. Publish it by adding the file
to the `bucket/` directory of a Scoop bucket (for example the maintainer's own
`hjosugi/scoop-bucket`) and pushing. `checkver` and `autoupdate` read the
GitHub releases, so a bucket can update itself with `scoop update` +
`checkver`; the hash comes from the release `SHA256SUMS`.

## Windows code signing (#15)

The release workflow signs the two Windows executables before zipping them when
the repository variable `WINDOWS_SIGNING` is `true`. It expects:

- secret `WINDOWS_CERT_PFX`: the base64 of a code-signing `.pfx` (for example
  from Azure Trusted Signing or an OV/EV certificate),
- secret `WINDOWS_CERT_PASSWORD`: its password.

With the variable unset the release is built and published unsigned, as before.
SmartScreen reputation is built per certificate, so a fresh OV certificate
warns until it has enough downloads; an EV certificate or Azure Trusted Signing
skips that. SignPath (free for OSS) can replace the step: submit
`touchpad-snap.exe` and `touchpad-snapw.exe` between the build and the zip step
and let its GitHub Action wait for the signed files.

## Notes

- Version bumping is automatic from the release checksums; publishing to the
  AUR, winget-pkgs and a Scoop bucket needs the respective account and stays
  manual.
- The Windows binaries are unsigned for now (#15); SmartScreen may warn.
