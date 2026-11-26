# Publishing on extensions.gnome.org

The extension is uploaded to [extensions.gnome.org](https://extensions.gnome.org/)
(EGO) by hand; EGO has no upload API. This is the complete checklist. Nothing
has been submitted yet for 0.7.0.

The release job builds the exact zip to upload as
`touchpad-snap@hjosugi.github.io.shell-extension.zip`, so the reviewed zip is
the one the CI matrix ran against. `task extension-zip` builds the same layout
locally.

## Before uploading: check the package

```console
$ scripts/check-ego.py 0.7.0
$ task extension-zip
$ unzip -l dist/touchpad-snap@hjosugi.github.io.shell-extension.zip
```

The zip must have `metadata.json` at its **root** (not inside a directory).
`check-ego.py` verifies the metadata, the shell-version list, the changelog
entry and the screenshot files, and that the zip layout is right when it
exists.

## What to upload

1. **Zip**: `dist/touchpad-snap@hjosugi.github.io.shell-extension.zip` from
   the release (asset name is exactly that).
2. **Screenshots**: at least one, PNG, at least 1280×720, no personal window
   titles, email addresses or notifications. Suggested set:
   - the HUD over two windows of one app (the jump overlay),
   - the settings window's binding list (optional; the extension itself is
     the bridge, the HUD is its visible part).
   Screenshots are uploaded on the EGO page, not inside the zip. Keep the
   originals in `gnome-extension/screenshots/` when you have them; the check
   script warns while that directory has no PNG. On a real session:
   `Print` opens GNOME's screenshot UI, or `gnome-screenshot -f shot.png`.
   A headless GNOME Shell refuses the screenshot D-Bus call
   (`org.freedesktop.DBus.Error.AccessDenied`), so this has to be a real
   session.
3. **Release notes**: the matching `## [0.7.0] - …` section from
   `CHANGELOG.md`. Say which `touchpad-snap` binary version it belongs to and
   any protocol change.
4. **Description**: `metadata.json`'s `description`. It must explain that this
   is a **bridge** and needs the companion `touchpad-snap` daemon; EGO review
   rejects extensions that look like a standalone app but need a binary.

## EGO checklist

- [ ] `shell-version` is the list the e2e matrix actually runs:
      `["46", "48", "49", "50", "51"]`. 47 is intentionally absent; add a CI
      matrix entry before adding 47.
- [ ] `metadata.json` has the final `name`, `uuid`, `url` and
      `version-name` (the release version, not an internal protocol number).
- [ ] No network requests, no file reads outside the extension directory, no
      `exec` of external programs. The extension only uses D-Bus, Clutter and
      St.
- [ ] No settings stored in GSettings: the daemon's `config.json` is the
      single source of truth, and grabs/claims are dropped when the daemon
      disconnects (`_release`).
- [ ] The extension does not hide unclaimed gestures: only claims registered
      by the daemon are stopped; everything else stays GNOME's.
- [ ] Screenshots are free of personal data and show the HUD/binding
      behaviour.
- [ ] `task extension-zip` and the release asset are byte-identical for the
      version being submitted.
- [ ] The version in `metadata.json` (`version-name`) and the release tag
      match; EGO shows this as the extension version.

## Upload flow

1. Sign in at <https://extensions.gnome.org/> with the GNOME account that
   should own the extension (the existing UUID
   `touchpad-snap@hjosugi.github.io` is already used by this project; do not
   create a second UUID).
2. **Upload Extension**, choose the zip, confirm the metadata shown.
3. Paste the CHANGELOG section as release notes.
4. Upload screenshots on the extension page.
5. Submit for review. The first submission needs review; later versions are
   usually published after a short automated/manual check.
6. After approval, update the README's install section with the EGO link (do
   not add the link before the extension is accepted).

## Review-facing summary (copy into the submission notes)

> touchpad-snap is a gesture and shortcut binder. The extension is a bridge
> only: it lists/activates windows, grabs the accelerators and swipe/pinch/
> hold claims the companion daemon asks for, and forwards those events on the
> session bus. All policy is in the daemon; the extension has no UI except a
> short overlay after a jump. It needs the touchpad-snap daemon
> (https://github.com/hjosugi/touchpad-snap), which is why the description
> names it. No network access, no file access, no command execution.

## After a release

The EGO version and the GitHub release are independent numbers but should be
kept equal. Upload every release that changes `extension.js`; if only the
daemon changed, the same extension zip still works across protocol versions
(older/newer extensions fall back with a warning). When the protocol changes,
say the minimum daemon version in the release notes.

The AUR package ships the same zip under
`/usr/share/gnome-shell/extensions/`; keep it in sync with the EGO upload.

## Install without EGO

`task install` copies the extension into
`~/.local/share/gnome-shell/extensions/` (log out and back in on Wayland),
then `gnome-extensions enable touchpad-snap@hjosugi.github.io`.
