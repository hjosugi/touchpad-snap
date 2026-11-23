# Publishing on extensions.gnome.org

The extension is uploaded to [extensions.gnome.org](https://extensions.gnome.org/)
(EGO) by hand; EGO has no upload API. Every release still produces the exact
zip to upload as `touchpad-snap@hjosugi.github.io.shell-extension.zip`, so the
reviewed zip is the one the release tests ran against.

## Checklist for a release

1. `task extension-zip` (or download the asset from the GitHub release) and
   check the zip has `metadata.json` at its root, not a directory:
   `unzip -l dist/touchpad-snap@hjosugi.github.io.shell-extension.zip`.
2. Upload it on EGO, paste the matching CHANGELOG section as the release
   notes, and say which `touchpad-snap` binary version it is for.
3. If the `shell-version` list changed, wait for the CI matrix first: EGO's
   shell versions should be the ones the e2e suite runs (46, 48, 49, 50 and
   the latest, currently 51). 47 is intentionally absent because nothing in
   CI runs it; add it back together with a matrix entry.

## Review guidelines

- The extension holds no policy: it lists and activates windows, grabs the
  hotkeys and swipe claims the daemon asks for, and forwards them over D-Bus.
  The description and the first paragraph of `README.md` say that the
  companion binary is required.
- It makes no network requests, reads no files outside its own directory and
  runs no commands.
- Settings are not stored in GSettings; the daemon's `config.json` is the
  single source of truth, and the extension forgets its grabs when the daemon
  disconnects.
- The `shell-version` list matches what CI tests.
- Screenshots for the EGO listing (at least one): a binding list or the HUD
  over two windows. Take them on a real session at 1x and 2x if possible;
  avoid showing personal window titles.

## Install without EGO

`task install` copies the extension into
`~/.local/share/gnome-shell/extensions/` (log out and back in on Wayland),
then `gnome-extensions enable touchpad-snap@hjosugi.github.io`.
