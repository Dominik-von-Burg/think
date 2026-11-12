#!/usr/bin/env bash
set -euo pipefail

binary=${1:?Usage: bash tests/linux-desktop.sh PATH_TO_THINK}
data_dir=$(mktemp -d)
pid=
wm_pid=
cleanup() {
    if [[ -n "$pid" ]]; then
        kill "$pid" 2>/dev/null || true
        wait "$pid" 2>/dev/null || true
    fi
    if [[ -n "$wm_pid" ]]; then
        kill "$wm_pid" 2>/dev/null || true
        wait "$wm_pid" 2>/dev/null || true
    fi
    rm -rf -- "$data_dir"
}
trap cleanup EXIT

fail() {
    printf '%s\n' "$1" >&2
    cat "$data_dir/openbox.log" "$data_dir/think.log" >&2 2>/dev/null || true
    exit 1
}

notebook="$data_dir/think/notebook.txt"
mkdir -p "$data_dir/think"
printf 'Existing note' > "$notebook"
printf 'Existing note\nWritten on Linux' > "$data_dir/expected.txt"

XDG_CONFIG_HOME="$data_dir" openbox --sm-disable > "$data_dir/openbox.log" 2>&1 &
wm_pid=$!
for ((attempt = 0; attempt < 50; attempt++)); do
    if xprop -root _NET_SUPPORTING_WM_CHECK | grep -q 'window id #'; then
        break
    fi
    sleep 0.1
done
xprop -root _NET_SUPPORTING_WM_CHECK | grep -q 'window id #' \
    || fail 'Openbox did not start.'

env -u WAYLAND_DISPLAY -u WAYLAND_SOCKET \
    XDG_DATA_HOME="$data_dir" DBUS_SESSION_BUS_ADDRESS="unix:path=$data_dir/no-session-bus" \
    WINIT_X11_SCALE_FACTOR=1 LIBGL_ALWAYS_SOFTWARE=1 \
    "$binary" > "$data_dir/think.log" 2>&1 &
pid=$!
window=$(timeout 15s xdotool search --sync --onlyvisible --all --pid "$pid" --name '^Think$') \
    || fail 'Think did not open its notebook window.'

# Openbox reports the size of the frame it added around the app. The top
# extent is positive when the native title bar is present.
timeout 5s xdotool windowactivate --sync "$window" || fail 'Could not activate the Think window.'
frame_top=$(xprop -id "$window" -notype _NET_FRAME_EXTENTS | awk -F ', *' 'NF == 4 {print $3}')
[[ "$frame_top" =~ ^[1-9][0-9]*$ ]] || fail 'Think did not receive a native title bar.'

# Click below the notebook toolbar and append with real X11 input.
xdotool mousemove --window "$window" 70 65 click 1 \
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

# Ask the window manager to close the app through its normal close action.
xdotool key alt+F4
for ((attempt = 0; attempt < 100; attempt++)); do
    if ! kill -0 "$pid" 2>/dev/null; then
        break
    fi
    sleep 0.1
done
kill -0 "$pid" 2>/dev/null && fail 'Think did not close on the window manager request.'
wait "$pid" || fail 'Think exited with an error.'
pid=
printf 'Think opened with a native title bar, autosaved the edited notebook, and closed on request.\n'
