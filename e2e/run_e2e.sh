#!/usr/bin/env bash
# End-to-end on Linux/X11: virtual display, file-backed mic, mock providers,
# a real key press, and a real paste into a Tk text box.
set -u
HERE="$(cd "$(dirname "$0")" && pwd)"
WORK="${WORK:-$(mktemp -d)}"
BIN="${BIN:-$HERE/../target/debug/murmur}"
export DISPLAY=:98 HOME="$WORK/home"
mkdir -p "$HOME/.config/app.murmur.dictation"
python3 -I "$HERE/make_audio.py" "$WORK/mic.raw"
# A clocked fake microphone: PulseAudio null sink; we play "speech" into it
# and Murmur records its monitor through ALSA's pulse plugin.
export XDG_RUNTIME_DIR="$WORK/run"; mkdir -p "$XDG_RUNTIME_DIR"; chmod 700 "$XDG_RUNTIME_DIR"
pulseaudio --daemonize=yes --exit-idle-time=-1 --disallow-exit -n \
  --load="module-native-protocol-unix" --load="module-null-sink sink_name=mic" --load="module-always-sink" >/dev/null 2>&1
sleep 1
pactl set-default-source mic.monitor
printf 'pcm.!default { type pulse }\nctl.!default { type pulse }\n' > "$HOME/.asoundrc"
cat > "$HOME/.config/app.murmur.dictation/settings.toml" <<TOML
history_limit = 50
[hotkey]
binding = "RightCtrl"
paste_last = "F9"
mode = "push_to_talk"
[stt]
provider = "custom"
base_url = "http://127.0.0.1:8799/v1"
[polish]
provider = "custom"
base_url = "http://127.0.0.1:8799/v1"
[dictionary]
terms = ["GitHub"]
[paste]
restore_delay_ms = 300
TOML
pkill -f '^Xvfb :98' 2>/dev/null; sleep 0.3
Xvfb :98 -screen 0 1280x800x24 +extension RECORD >/dev/null 2>&1 & XV=$!
python3 -I "$HERE/mock_providers.py" 8799 "$WORK/requests.log" & MOCK=$!
sleep 1
"${PYTHON_TK:-/usr/bin/python3.12}" -I "$HERE/target_app.py" "$WORK/field.txt" & TK=$!
MURMUR_NO_KEYCHAIN=1 RUST_LOG=info "$BIN" > "$WORK/app.log" 2>&1 & APP=$!
# Wait for the hotkey to be live instead of guessing (CI machines are slower).
wait_for() { local t=$1; shift; for _ in $(seq 1 $((t * 10))); do eval "$@" && return 0; sleep 0.1; done; return 1; }
wait_for 60 'grep -q "ready: hold" "$WORK/app.log"' || echo "x app never reported ready"
wait_for 20 'xdotool search --name "general - Slack" >/dev/null 2>&1' || echo "x target window missing"
sleep 1
WID=$(xdotool search --name "general - Slack" | head -1)
xdotool windowactivate --sync "$WID" 2>/dev/null || xdotool windowfocus --sync "$WID"
xdotool mousemove --window "$WID" 50 50 click 1
sleep 0.5
printf 'original clipboard' | xclip -selection clipboard -loops 0 &
xdotool keydown Control_R; sleep 0.3
pacat --playback --device=mic --format=s16le --rate=48000 --channels=2 "$WORK/mic.raw"
sleep 0.2; xdotool keyup Control_R
EXPECT="Draft: Can you push it to GitHub before Friday? "
wait_for 30 '[[ "$(cat "$WORK/field.txt" 2>/dev/null)" == "$EXPECT" ]]'
sleep 1
FIRST="$(cat "$WORK/field.txt" 2>/dev/null)"
# Paste-last: the shortcut should type the same dictation again.
xdotool key F9
wait_for 15 '[[ "$(cat "$WORK/field.txt" 2>/dev/null)" == "$EXPECT${EXPECT#Draft: }" ]]'
sleep 1
FIELD="$(cat "$WORK/field.txt" 2>/dev/null)"
CLIP="$(timeout 2 xclip -selection clipboard -o 2>/dev/null)"
echo "--- requests"; cat "$WORK/requests.log" 2>/dev/null
echo "--- field: [$FIELD]"
echo "--- clipboard after: [$CLIP]"
kill $APP $TK $MOCK $XV 2>/dev/null; pulseaudio --kill 2>/dev/null; wait 2>/dev/null
ok=1
[[ "$FIRST" == "Draft: Can you push it to GitHub before Friday? " ]] || { echo "x text not inserted as expected: [$FIRST]"; ok=0; }
[[ "$FIELD" == "Draft: Can you push it to GitHub before Friday? Can you push it to GitHub before Friday? " ]] || { echo "x paste-last didn't re-insert the dictation"; ok=0; }
[[ "$CLIP" == "original clipboard" ]] || { echo "x clipboard not restored"; ok=0; }
grep -q '"system_has_work_chat": true' "$WORK/requests.log" || { echo "x app not classified as work chat"; ok=0; }
grep -q '"vocab": true' "$WORK/requests.log" || { echo "x vocabulary not sent to STT"; ok=0; }
if [[ $ok == 1 ]]; then echo "E2E PASS"; exit 0; fi
echo "E2E FAIL. App log:"; grep -vE "ALSA|[Jj]ack|Cannot connect|libEGL|dbind" "$WORK/app.log" | tail -40; exit 1
