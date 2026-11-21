#!/usr/bin/env bash
# The mechanical part of renaming window-jump to touchpad-snap (#31).
#
#   scripts/rename-to-touchpad-snap.sh
#
# Run it from a clean checkout. It moves the paths that carry the name and
# rewrites every spelling of it in tracked text files:
#
#   window-jump             touchpad-snap     (binary, crate, extension UUID,
#                                              config directory, CSS classes)
#   window_jump             touchpad_snap     (the Rust library)
#   WindowJump              TouchpadSnap      (type names)
#   org.hjosugi.WindowJump  io.github.hjosugi.TouchpadSnap   (D-Bus name,
#   /org/hjosugi/...        /io/github/hjosugi/...            path, interface)
#
# CHANGELOG.md is left alone: it records what the releases were called. What
# a sed cannot do (reading the old config directory, removing the old
# autostart entry, noticing the old extension) is a separate commit.
#
# Running it twice is harmless. This script deletes nothing but itself is
# meant to be removed by the pull request that applies it.
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"

if [[ -n "$(git status --porcelain --untracked-files=no)" ]]; then
    echo "the working tree has changes; commit or stash them first" >&2
    exit 1
fi

move() {
    if [[ -e "$1" ]]; then
        git mv "$1" "$2"
    fi
}
move "gnome-extension/window-jump@hjosugi.github.io" "gnome-extension/touchpad-snap@hjosugi.github.io"
move "src/bin/window-jumpw.rs" "src/bin/touchpad-snapw.rs"

# Any other tracked path with the name in it would be a new one this script
# does not know yet: stop rather than leave it behind.
if left=$(git ls-files | grep -i -E 'window[-_]?jump'); then
    echo "paths this script does not know how to move:" >&2
    echo "$left" >&2
    exit 1
fi

self="scripts/rename-to-touchpad-snap.sh"
mapfile -t files < <(git grep -l -I -i -E 'window[-_]?jump|org[./]hjosugi' -- . ":!CHANGELOG.md" ":!$self" || true)
if ((${#files[@]})); then
    sed -i -E \
        -e 's#org\.hjosugi\.WindowJump#io.github.hjosugi.TouchpadSnap#g' \
        -e 's#org/hjosugi/WindowJump#io/github/hjosugi/TouchpadSnap#g' \
        -e 's#WindowJump#TouchpadSnap#g' \
        -e 's#window-jump#touchpad-snap#g' \
        -e 's#window_jump#touchpad_snap#g' \
        "${files[@]}"
fi

# Whatever is left is a spelling nobody planned for.
if left=$(git grep -n -I -i -E 'window[-_ ]?jump|org[./]hjosugi' -- . ":!CHANGELOG.md" ":!$self"); then
    echo "spellings of the old name this script does not know:" >&2
    echo "$left" >&2
    exit 1
fi
if command -v cargo >/dev/null; then
    cargo fmt # the new name is one letter longer: lines reflow
fi
echo "renamed in ${#files[@]} file(s); review with: git status && git diff --stat"
