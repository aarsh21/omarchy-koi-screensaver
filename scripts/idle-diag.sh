#!/usr/bin/env bash
# Diagnostic + capture: the compositor never reports idle while the 8K wireless mouse is streaming
# motion, so disable that pointer for twenty seconds, let the real idle service open the screensaver,
# capture it, then put the pointer and every setting back.
set -uo pipefail

OUT=${KOI_PROOF_DIR:-$HOME/.amp/in/artifacts/koi-proof}
CLS=org.omarchy.screensaver
export C="$CLS"
SHELL_JSON="$HOME/.config/omarchy/shell.json"
MOUSE=compx-wireless-mouse-8k-nano-dongle-l-mouse
LOG="$OUT/idle-capture.log"

mkdir -p "$OUT"
cp "$SHELL_JSON" /tmp/koi-diag-shell.json
restore_awake=0

cleanup() {
  hyprctl eval "hl.device({ name = \"$MOUSE\", enabled = true })" >/dev/null 2>&1
  cp /tmp/koi-diag-shell.json "$SHELL_JSON"
  [[ $restore_awake == 1 ]] && omarchy-toggle-idle stay-awake >/dev/null 2>&1
}
trap cleanup EXIT

if omarchy-toggle-idle status | grep -q '"enabled":true'; then
  omarchy-toggle-idle allow-idle >/dev/null
  restore_awake=1
fi
jq '.idle.screensaver = 1' "$SHELL_JSON" >/tmp/koi-diag.json && mv /tmp/koi-diag.json "$SHELL_JSON"

echo "== $(date -Is) disabling $MOUSE for the test" | tee -a "$LOG"
hyprctl eval "hl.device({ name = \"$MOUSE\", enabled = false })" 2>&1 | tee -a "$LOG"
sleep 1
wtype -M shift -m shift 2>/dev/null || true

for _ in $(seq 1 40); do
  if [[ $(hyprctl clients -j | jq '[.[] | select(.class == env.C)] | length') -gt 0 ]]; then
    echo "screensaver opened by the idle service, no input at all" | tee -a "$LOG"
    sleep 2
    grim "$OUT/koi-idle-live.png"
    hyprctl clients -j | jq -c '.[] | select(.class==env.C) | {class, fullscreen, size}' | tee -a "$LOG"
    pid=$(hyprctl clients -j | jq -r '.[] | select(.class==env.C) | .pid')
    echo "child exe: $(readlink "/proc/$(pgrep -P "$pid" | head -1)/exe")" | tee -a "$LOG"
    journalctl --user -b -g "omarchy idle" --no-pager -n 10 | tail -6 | tee -a "$LOG"
    wtype -k x 2>/dev/null || true
    sleep 1
    exit 0
  fi
  sleep 0.5
done
echo "no idle event even with the mouse disabled: $(omarchy-shell idle status | jq -c '{idle,lastEvent}')" | tee -a "$LOG"
exit 1
