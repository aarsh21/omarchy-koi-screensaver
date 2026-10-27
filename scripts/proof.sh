#!/usr/bin/env bash
# End-to-end proof that the koi pond is the Omarchy screensaver.
#
# Everything runs against the real compositor, the real launcher and a real terminal, and writes
# screenshots, logs and state dumps into $KOI_PROOF_DIR (default
# ~/.amp/in/artifacts/koi-proof). Run it from the repository root.
set -euo pipefail

ROOT=$(cd "$(dirname "$0")/.." && pwd)
OUT=${KOI_PROOF_DIR:-$HOME/.amp/in/artifacts/koi-proof}
STOCK=/usr/bin/omarchy-screensaver
INSTALLED=/usr/local/bin/omarchy-screensaver
CLS=org.omarchy.screensaver
export C="$CLS"
TRANSCRIPT="$OUT/transcript.log"

SHELL_JSON="$HOME/.config/omarchy/shell.json"
workdir=$(mktemp -d)
SHELL_JSON_BAK="$workdir/shell.json"
restored_stay_awake=0
focus_target=$(hyprctl activewindow -j | jq -r '.address // empty')
stock_hash=$(sha256sum "$STOCK" | cut -d' ' -f1)

# The idle phase shortens the user's idle timeout and may release stay-awake for the test; put both
# back even if the proof is interrupted.
cleanup() {
  if [[ -f $SHELL_JSON_BAK ]]; then
    local tmp original
    original=$(jq '.idle.screensaver' "$SHELL_JSON_BAK")
    tmp=$(mktemp "$(dirname "$SHELL_JSON")/koi-proof.XXXXXX")
    jq --argjson original "$original" '.idle.screensaver = $original' "$SHELL_JSON" >"$tmp" &&
      mv "$tmp" "$SHELL_JSON"
  fi
  if [[ $restored_stay_awake == 1 ]]; then
    omarchy-toggle-idle stay-awake >/dev/null 2>&1 || true
    restored_stay_awake=0
  fi
  rm -rf "$workdir"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

mkdir -p "$OUT"
: >"$TRANSCRIPT"

log() { printf '%s\n' "$*" | tee -a "$TRANSCRIPT"; }
say() { printf '\n=== %s ===\n' "$*" | tee -a "$TRANSCRIPT"; }
fail() { log "  FAIL: $*"; exit 1; }

cls_windows() {
  hyprctl clients -j | jq '[.[] | select(.class == env.C)] | length'
}
has_window() { [[ $(cls_windows) -gt 0 ]]; }
no_window() { [[ $(cls_windows) -eq 0 ]]; }
active_class() { hyprctl activewindow -j | jq -r '.class // ""'; }

effect_pid() {
  hyprctl clients -j | jq -r '[.[] | select(.class==env.C and .title=="Koi pond")][0].pid'
}

wait_for() { # wait_for <seconds> <predicate>
  local deadline=$((SECONDS + $1))
  shift
  while ((SECONDS < deadline)); do
    if "$@"; then return 0; fi
    sleep 0.25
  done
  return 1
}

shot() { grim "$OUT/$1"; log "  screenshot: $OUT/$1"; }

capture_frames() { # capture_frames <prefix> <count> <delay>
  local prefix=$1 count=$2 delay=$3 i
  for ((i = 0; i < count; i++)); do
    grim "$OUT/$prefix-$i.png"
    sleep "$delay"
  done
  log "  captured $count frames -> $OUT/$prefix-*.png"
}

leave_screensaver() {
  no_window && return 0
  [[ $(active_class) == "$CLS" ]] || return 1
  wtype -k x 2>/dev/null || true
  if wait_for 6 no_window; then
    log "  window closed after a keypress"
    return 0
  fi
  log "  !! window still open after keypress"
  return 1
}

# ── 0. what is installed ───────────────────────────────────────────────────────────────────────
say "0. which binary the compositor resolves"
{
  echo "date:            $(date -Is)"
  echo "which:           $(command -v omarchy-screensaver)"
  echo "shell PATH:      $PATH"
  echo "compositor PATH: $(tr '\0' '\n' <"/proc/$(pgrep -x Hyprland | head -1)/environ" | sed -n 's/^PATH=//p')"
  echo "stock:           $STOCK  md5=$(md5sum "$STOCK" | cut -d' ' -f1)"
  echo "installed:       $INSTALLED  md5=$(md5sum "$INSTALLED" 2>/dev/null | cut -d' ' -f1)"
  echo "installed file:  $(file -b "$INSTALLED" 2>/dev/null)"
  echo "version:         $("$INSTALLED" --version 2>/dev/null)"
} | tee -a "$TRANSCRIPT"

# ── 1. before: the stock ttfx screensaver ─────────────────────────────────────────────────────
say "1. before: stock screensaver (ttfx)"
if [[ "${KOI_SKIP_STOCK:-0}" != 1 ]]; then
  (foot --app-id="$CLS" --config=/usr/share/omarchy/default/foot/screensaver.ini -e "$STOCK" \
    >/dev/null 2>&1 </dev/null &)
  if wait_for 8 has_window; then
    sleep 2.5
    shot stock-screensaver.png
    log "  window: $(hyprctl clients -j | jq -c '.[] | select(.class==env.C) | {class, fullscreen, size}')"
    log "  process: $(ps -eo args | grep -E '[t]tfx' | head -1)"
    leave_screensaver
    sleep 1
  else
    fail "stock screensaver window never appeared"
  fi
fi

# ── 2. install the koi pond ───────────────────────────────────────────────────────────────────
say "2. install"
"$ROOT/install.sh" install 2>&1 | sed 's/^/  /' | tee -a "$TRANSCRIPT"
log "  resolves to: $(command -v omarchy-screensaver)"

# ── 3. real path: Omarchy's own launcher, which the idle service runs ────────────────────────
say "3. launch through omarchy-launch-screensaver (the idle service's command)"
omarchy-launch-screensaver force >/dev/null 2>&1
if wait_for 15 has_window; then
  log "  active window: $(hyprctl activewindow -j | jq -c '{class,title,fullscreen}')"
  sleep 2
  koi_child=$(effect_pid)
  log "  native process:"
  ps -o pid,ppid,pcpu,etime,args -p "$koi_child" | sed 's/^/    /' | tee -a "$TRANSCRIPT"
  [[ $(cls_windows) -eq $(hyprctl monitors -j | jq length) ]]
  log "  effect pid $koi_child"
  log "  child exe: $(readlink "/proc/$koi_child/exe")"
  [[ $(readlink "/proc/$koi_child/exe") == /usr/local/bin/koi-screensaver ]]
  log "  shim: $(readlink /usr/share/omarchy/bin/omarchy-screensaver)"
  recorder=""
  if command -v gpu-screen-recorder >/dev/null; then
    monitor=$(hyprctl monitors -j | jq -r '.[] | select(.focused) | .name')
    gpu-screen-recorder -w "$monitor" -f 60 -c mp4 -k h264 -cursor no \
      -o "$OUT/koi-native.mp4" >"$OUT/recording.log" 2>&1 &
    recorder=$!
  fi
  capture_frames koi 10 0.35
  if [[ -n $recorder ]]; then
    sleep 3
    kill -INT "$recorder" 2>/dev/null || true
    wait "$recorder" || log "  !! native recording failed; see recording.log"
  fi
  convert -delay 35 -loop 0 "$OUT"/koi-[0-9].png "$OUT/koi-motion.gif"
  log "  animation: $OUT/koi-motion.gif ($(du -h "$OUT/koi-motion.gif" | cut -f1))"
  shot koi-live.png
  log "  cpu: $(ps -o pcpu=,pmem=,etime= -p "$koi_child" | tr -s ' ' | sed 's/^/cpu%,mem%,runtime:/')"
  leave_screensaver
  sleep 1
  log "  after exit: windows=$(cls_windows) koi_processes=$(pgrep -x koi-screensaver | tr '\n' ' ')"
else
  fail "screensaver window never appeared"
fi

# ── 4. installed native renderer, including automatic feeding ─────────────────────────────────
say "4. fps, native pixel size, and feeding from the installed binary"
rm -f "$OUT/stats.jsonl"
KOI_SEED=42 KOI_STATS="$OUT/stats.jsonl" KOI_EXIT_AFTER=32 \
  omarchy-launch-screensaver force >/dev/null 2>&1
wait_for 8 has_window || fail "stats window never appeared"
sleep 24
shot koi-feeding.png
sleep 10
log "  stats:"
sed 's/^/    /' "$OUT/stats.jsonl" | tee -a "$TRANSCRIPT"
jq -es 'length >= 5 and all(.[]; .backend == "wayland" and .fps > 0) and any(.[]; .pellets > 0)' "$OUT/stats.jsonl" >/dev/null ||
  fail "native renderer statistics or automatic feeding missing"
leave_screensaver || true
sleep 1

# ── 5. terminal restored on exit (real pty, no compositor involved) ──────────────────────────
say "5. pty lifecycle: alt screen, raw mode and cursor restored"
KOI_FOCUS_CLASS="" KOI_EXIT_AFTER=2 script -q -e -c \
  'before=$(stty -g); /usr/local/bin/omarchy-screensaver --terminal; result=$?; test "$result" = 0 && test "$before" = "$(stty -g)"' \
  "$OUT/pty.log" >/dev/null 2>&1
log "  exit code:        $?"
log "  enter alt screen: $(grep -c $'\x1b\[?1049h' "$OUT/pty.log")"
log "  leave alt screen: $(grep -c $'\x1b\[?1049l' "$OUT/pty.log")"
log "  cursor restored:  $(grep -c $'\x1b\[?25h' "$OUT/pty.log")"
log "  background set:   $(grep -c $'\x1b]11;rgb' "$OUT/pty.log")"
log "  bytes captured:   $(wc -c <"$OUT/pty.log")"
for sequence in $'\x1b[?1049h' $'\x1b[?1049l' $'\x1b[?25h' $'\x1b]11;rgb'; do
  grep -Fq "$sequence" "$OUT/pty.log"
done

# ── 6. leaves when its window loses focus (stock behaviour) ─────────────────────────────────
say "6. focus loss closes the screensaver"
omarchy-launch-screensaver force >/dev/null 2>&1
if wait_for 15 has_window; then
  sleep 2
  log "  before stealing focus: windows=$(cls_windows) active=$(active_class)"
  [[ -n $focus_target ]] || fail "start the proof with a normal application window focused"
  hyprctl dispatch "hl.dsp.focus({ window = \"address:$focus_target\" })" >/dev/null 2>&1 ||
    hyprctl dispatch focuswindow "address:$focus_target" >/dev/null
  sleep 0.5
  log "  after stealing focus:  active=$(active_class)"
  if wait_for 8 no_window; then
    log "  screensaver exited on its own"
  else
    fail "screensaver stayed open after losing focus"
  fi
else
  fail "screensaver window never appeared"
fi

# ── 7. the real idle trigger ─────────────────────────────────────────────────────────────────
say "7. idle trigger: the shell's idle service starts the screensaver"
if [[ "${KOI_SKIP_IDLE:-0}" == 1 ]]; then
  log "  skipped (KOI_SKIP_IDLE=1)"
else
  shell_json="$SHELL_JSON"
  cp "$shell_json" "$SHELL_JSON_BAK"
  original_timeout=$(jq -r '.idle.screensaver' "$shell_json")

  # Waiting out a 150s idle timeout is unreliable on a machine someone is using, so shorten the
  # timeout for the test, let the real idle service fire, then put the user's value back.
  if omarchy-toggle-idle status | grep -q '"enabled":true'; then
    log "  stay-awake was ON (it suppresses every idle timer, the stock screensaver included); allowing idle for the test"
    omarchy-toggle-idle allow-idle >/dev/null
    restored_stay_awake=1
  fi

  tmp=$(mktemp)
  jq '.idle.screensaver = 6' "$shell_json" >"$tmp" && mv "$tmp" "$shell_json"
  log "  idle.screensaver ${original_timeout}s -> 6s for the test"
  sleep 1
  log "  shell sees: $(omarchy-shell idle status | jq -c '{screensaver, enabled, stayAwake}')"

  # The idle service starts a cycle on the compositor's active -> idle transition. Send one real key
  # (Shift on its own types nothing) so the compositor is "active", then generate no input at all:
  # every keystroke would restart the clock, and this machine's user may be at the keyboard.
  wtype -M shift -m shift 2>/dev/null || true
  wait_started=$SECONDS
  log "  waiting up to ${KOI_IDLE_WAIT:-300}s for a 6s lull"
  got=0
  if wait_for "${KOI_IDLE_WAIT:-300}" has_window; then
    got=1
    log "  screensaver appeared after $((SECONDS - wait_started))s of idle"
  fi

  if [[ $got == 1 ]]; then
    sleep 2
    shot koi-idle.png
    koi_child=$(effect_pid)
    log "  window: $(hyprctl clients -j | jq -c '.[] | select(.class==env.C) | {class, fullscreen, size}')"
    log "  child exe: $(readlink "/proc/$koi_child/exe")"
    log "  idle service log:"
    journalctl --user -b -g "omarchy idle" --no-pager -n 8 | sed 's/^/    /' | tee -a "$TRANSCRIPT"
    leave_screensaver
    sleep 1
  else
    fail "no idle launch observed; check input, inhibitors, and Omarchy's idle service"
  fi

  cleanup
  log "  idle.screensaver restored to $(jq -r '.idle.screensaver' "$shell_json")s"
  log "  stay-awake restored: $(omarchy-toggle-idle status)"
fi

# ── 8. control checks ───────────────────────────────────────────────────────────────────────
say "8. control checks"
[[ $(sha256sum "$STOCK" | cut -d' ' -f1) == "$stock_hash" ]]
no_window
if pgrep -x koi-screensaver >/dev/null; then fail "leftover koi processes"; fi
log "  stock binary still present and unchanged: $(md5sum "$STOCK")"
log "  installed binary:                         $(md5sum "$INSTALLED")"
log "  idle timeout:                             $(jq -r '.idle.screensaver' "$HOME/.config/omarchy/shell.json") seconds"
if omarchy-toggle-enabled screensaver-off 2>/dev/null; then
  log "  screensaver toggle:                       off (screensaver disabled)"
else
  log "  screensaver toggle:                       on"
fi
log "  leftover koi processes:                   0"
log "  leftover windows:                         $(cls_windows)"

say "all requested phases passed (check explicit skips above)"
log "artefacts in $OUT"
