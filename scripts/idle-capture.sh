#!/usr/bin/env bash
# Passive capture of a real idle-triggered screensaver.
#
# The idle service only starts a cycle on the compositor's active -> idle transition, and any input
# on the machine restarts that clock, so this waits without touching anything and captures the first
# screensaver window that the idle service opens. Restores the idle timeout and stay-awake state.
set -uo pipefail

OUT=${KOI_PROOF_DIR:-$HOME/.amp/in/artifacts/koi-proof}
CLS=org.omarchy.screensaver
export C="$CLS"
SHELL_JSON="$HOME/.config/omarchy/shell.json"
BAK=/tmp/koi-idle-shell.json.bak
LIMIT=${1:-600}

mkdir -p "$OUT"
cp "$SHELL_JSON" "$BAK"
original=$(jq -r '.idle.screensaver' "$SHELL_JSON")
restore_awake=0

cleanup() {
  jq --argjson t "$original" '.idle.screensaver = $t' "$SHELL_JSON" >/tmp/koi-idle-restore.json &&
    mv /tmp/koi-idle-restore.json "$SHELL_JSON"
  [[ $restore_awake == 1 ]] && omarchy-toggle-idle stay-awake >/dev/null 2>&1
}
trap cleanup EXIT

if omarchy-toggle-idle status | grep -q '"enabled":true'; then
  omarchy-toggle-idle allow-idle >/dev/null
  restore_awake=1
fi
jq '.idle.screensaver = 6' "$SHELL_JSON" >/tmp/koi-idle-set.json && mv /tmp/koi-idle-set.json "$SHELL_JSON"

echo "waiting up to ${LIMIT}s for the idle service, idle.screensaver=$(jq -r '.idle.screensaver' "$SHELL_JSON")s" | tee "$OUT/idle-capture.log"
start=$SECONDS
while ((SECONDS - start < LIMIT)); do
  if [[ $(hyprctl clients -j | jq '[.[] | select(.class == env.C)] | length') -gt 0 ]]; then
    echo "screensaver appeared after $((SECONDS - start))s" | tee -a "$OUT/idle-capture.log"
    sleep 2
    grim "$OUT/koi-idle-live.png"
    hyprctl clients -j | jq -c '.[] | select(.class==env.C) | {class, fullscreen, size}' | tee -a "$OUT/idle-capture.log"
    pid=$(hyprctl clients -j | jq -r '.[] | select(.class==env.C) | .pid')
    echo "child exe: $(readlink "/proc/$(pgrep -P "$pid" | head -1)/exe")" | tee -a "$OUT/idle-capture.log"
    journalctl --user -b -g "omarchy idle" --no-pager -n 8 | tail -4 | tee -a "$OUT/idle-capture.log"
    wtype -k x 2>/dev/null || true
    sleep 1
    exit 0
  fi
  sleep 0.5
done
echo "no idle-triggered screensaver within ${LIMIT}s (input kept the compositor active)" | tee -a "$OUT/idle-capture.log"
exit 1
