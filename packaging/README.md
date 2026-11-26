# Package manifests (AUR, winget, Scoop)

Everything needed to publish touchpad-snap outside GitHub is prepared in this
directory. **Nothing here has been submitted to AUR, winget-pkgs or a Scoop
bucket yet**; that needs the maintainer's accounts. Every release regenerates
the files from the published `SHA256SUMS`, so the next release is a copy/paste
job.

Check a release before copying anything:

```console
$ gh release download v0.7.0 --pattern SHA256SUMS --dir /tmp/sums
$ scripts/update-packaging.py 0.7.0 --sums /tmp/sums/SHA256SUMS   # regenerate
$ scripts/check-packaging.py  0.7.0 --sums /tmp/sums/SHA256SUMS   # verify
packaging for 0.7.0 is consistent
```

The release workflow runs `update-packaging.py` after publishing and uploads
the result as the `packaging-v<version>` artifact. `check-packaging.py` is not
run in CI (it needs the release's checksums); run it before submitting.

## What is prepared for 0.7.0

| Target | File(s) |
|---|---|
| AUR | `aur/touchpad-snap-bin/PKGBUILD`, `aur/touchpad-snap-bin/.SRCINFO` |
| winget | `winget/manifests/h/hjosugi/touchpad-snap/0.7.0/*.yaml` (3 files) |
| Scoop | `scoop/bucket/touchpad-snap.json` |

Hashes are the release's SHA256SUMS. The AUR package installs the daemon, the
systemd user unit and the GNOME extension under
`/usr/share/gnome-shell/extensions`; it does not enable the extension (the
user does that once, after logging in again).

## AUR (`touchpad-snap-bin`)

Prepared files: `PKGBUILD` (x86_64 + aarch64, musl archives, extension zip) and
the generated `.SRCINFO`. `makepkg` is not installed in this repository's
environment, so regenerate `.SRCINFO` after any PKGBUILD edit on Arch:

```console
$ cd packaging/aur/touchpad-snap-bin
$ makepkg --printsrcinfo > .SRCINFO
$ makepkg -f                       # builds in a clean chroot-like directory
$ namcap PKGBUILD                 # optional lint
$ git clone ssh://aur@aur.archlinux.org/touchpad-snap-bin.git /tmp/aur
$ cp PKGBUILD .SRCINFO /tmp/aur/
$ cd /tmp/aur
$ git add PKGBUILD .SRCINFO
$ git commit -m "Update to 0.7.0"
$ git push
```

Prerequisites: an AUR account and an SSH key added to it
(<https://aur.archlinux.org/account>), `base-devel` and `unzip` installed.
Notes:

- The package is `*-bin` because it ships prebuilt musl binaries; do not strip
  them (`options=('!strip')`).
- It provides/conflicts with `touchpad-snap` and `window-jump-bin` (the old
  name).
- The extension inside the package is the same zip uploaded to
  extensions.gnome.org; keep both at the same version.
- Do not submit a package whose `pkgver` does not have a matching GitHub
  release and entry in the release's `SHA256SUMS`.

## winget (`hjosugi.touchpad-snap`)

Prepared files: the three manifests under
`winget/manifests/h/hjosugi/touchpad-snap/0.7.0/`. Validate on Windows:

```powershell
winget validate --manifest .\packaging\winget\manifests\h\hjosugi\touchpad-snap\0.7.0
```

or via `wingetcreate` (recommended, it also opens the PR):

```powershell
winget install Microsoft.WingetCreate
wingetcreate update hjosugi.touchpad-snap --version 0.7.0 `
  --urls https://github.com/hjosugi/touchpad-snap/releases/download/v0.7.0/touchpad-snap-v0.7.0-x86_64-pc-windows-msvc.zip `
         https://github.com/hjosugi/touchpad-snap/releases/download/v0.7.0/touchpad-snap-v0.7.0-aarch64-pc-windows-msvc.zip `
  --submit
```

Manual PR path: fork <https://github.com/microsoft/winget-pkgs>, copy the
version folder unchanged to `manifests/h/hjosugi/touchpad-snap/0.7.0/`, commit
on a branch named `hjosugi.touchpad-snap-0.7.0`, and open a PR against
`microsoft/winget-pkgs:master`. The first PR may prompt for a CLA.

The installer is the portable zip: winget installs `touchpad-snap.exe` as
`touchpad-snap` and the console-less `touchpad-snapw.exe` as
`touchpad-snapw`. The zip also carries `touchpad-snap-settings.exe` for the
tray's *Settings…* item (winget does not add a command for it, so start it
from the tray). The Windows binaries are unsigned until #15 is set up;
SmartScreen may warn. Check the PR's validation log and fix the manifests if
the schema changes (manifest version is 1.6.0).

## Scoop

Prepared file: `scoop/bucket/touchpad-snap.json`, ready for a bucket's
`bucket/` directory. The intended bucket is `hjosugi/scoop-bucket` (create it
once on GitHub, public, with `bucket/`):

```console
$ git clone https://github.com/hjosugi/scoop-bucket /tmp/scoop
$ mkdir -p /tmp/scoop/bucket
$ cp packaging/scoop/bucket/touchpad-snap.json /tmp/scoop/bucket/
$ cd /tmp/scoop && git add bucket/touchpad-snap.json
$ git commit -m "touchpad-snap 0.7.0" && git push
```

Users then run `scoop bucket add hjosugi https://github.com/hjosugi/scoop-bucket`
and `scoop install touchpad-snap`. `checkver` reads GitHub releases and
`autoupdate` uses the release `SHA256SUMS`, so later versions can be updated
with `scoop update` and a `checkver` bump.

## Exact submission checklist

For a release `X.Y.Z`:

1. `scripts/update-packaging.py X.Y.Z` (or download the `packaging-vX.Y.Z`
   artifact) and `scripts/check-packaging.py X.Y.Z`.
2. AUR: regenerate `.SRCINFO`, commit `PKGBUILD` + `.SRCINFO` to
   `touchpad-snap-bin`.
3. winget: validate and open the PR for `hjosugi.touchpad-snap` `X.Y.Z`.
4. Scoop: copy `bucket/touchpad-snap.json` to the bucket and push.
5. GNOME extension: upload the release's shell-extension zip to
   extensions.gnome.org (`../gnome-extension/PUBLISHING.md`).
6. Signing (when a certificate exists): see `SIGNING.md`; the release must be
   rebuilt after enabling it so the zips contain signed executables.

## Windows code signing (#15)

The release workflow signs the two daemon executables before zipping them when
the repository variable `WINDOWS_SIGNING` is `true`. It expects:

- secret `WINDOWS_CERT_PFX`: the base64 of a code-signing `.pfx`,
- secret `WINDOWS_CERT_PASSWORD`: its password.

With the variable unset the release is built and published unsigned, as
before. See `SIGNING.md` for the certificate options, exact commands and how
to verify a signed release.
