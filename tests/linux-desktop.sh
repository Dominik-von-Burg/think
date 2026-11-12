#!/usr/bin/env bash
set -euo pipefail

binary=${1:?Usage: bash tests/linux-desktop.sh PATH_TO_THINK}
data_dir=$(mktemp -d)
pid=
cleanup() {
    if [[ -n "$pid" ]]; then
        kill "$pid" 2>/dev/null || true
        wait "$pid" 2>/dev/null || true
    fi
    rm -rf -- "$data_dir"
}
trap cleanup EXIT

fail() {
    printf '%s\n' "$1" >&2
    cat "$data_dir/think.log" >&2
    exit 1
}

notebook="$data_dir/think/notebook.txt"
mkdir -p "$data_dir/think"
printf 'Existing note' > "$notebook"
printf 'Existing note\nWritten on Linux' > "$data_dir/expected.txt"

env -u WAYLAND_DISPLAY -u WAYLAND_SOCKET \
    XDG_DATA_HOME="$data_dir" DBUS_SESSION_BUS_ADDRESS="unix:path=$data_dir/no-session-bus" \
    WINIT_X11_SCALE_FACTOR=1 LIBGL_ALWAYS_SOFTWARE=1 \
    "$binary" > "$data_dir/think.log" 2>&1 &
pid=$!
window=$(timeout 15s xdotool search --sync --onlyvisible --all --pid "$pid" --name '^Think$') \
    || fail 'Think did not open its notebook window.'

# Click below the title bar, then append to the notebook with real X11 input.
timeout 5s xdotool windowfocus --sync "$window" mousemove --window "$window" 70 65 click 1 \
    || fail 'Could not focus the Think editor.'
xdotool key ctrl+End Return
xdotool type 'Written on Linux'

for ((attempt = 0; attempt < 100; attempt++)); do
    if cmp -s "$data_dir/expected.txt" "$notebook"; then
        break
    fi
    sleep 0.1
done
if ! cmp -s "$data_dir/expected.txt" "$notebook"; then
    diff -u "$data_dir/expected.txt" "$notebook" >&2 || true
    fail 'Think did not autosave the edited notebook.'
fi

geometry=$(xdotool getwindowgeometry --shell "$window")
width=$(sed -n 's/^WIDTH=//p' <<<"$geometry")
height=$(sed -n 's/^HEIGHT=//p' <<<"$geometry")

# X11 excludes the rounded corners from the window's input region.
for x in 0 "$((width - 1))"; do
    for y in 0 "$((height - 1))"; do
        location=$(xdotool mousemove --window "$window" "$x" "$y" getmouselocation --shell)
        if [[ $(sed -n 's/^WINDOW=//p' <<<"$location") == "$window" ]]; then
            fail "Think's window still covers corner ($x, $y)."
        fi
    done
done

# At 1x scale the Linux close button is centered 18 px from the right edge.
xdotool mousemove --window "$window" "$((width - 18))" 18 click 1
for ((attempt = 0; attempt < 100; attempt++)); do
    if ! kill -0 "$pid" 2>/dev/null; then
        break
    fi
    sleep 0.1
done
kill -0 "$pid" 2>/dev/null && fail 'Think did not close from its title bar.'
wait "$pid" || fail 'Think exited with an error.'
pid=
printf 'Think opened with rounded corners, autosaved the edited notebook, and closed from its title bar.\n'
