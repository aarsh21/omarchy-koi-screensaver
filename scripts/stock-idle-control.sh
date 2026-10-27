#!/usr/bin/env bash
# Control experiment: does the *stock* ttfx screensaver come up on idle right now?
#
# Same idle timeout, same stay-awake state, same watcher as the koi run. If the stock one does not
# appear either, the idle notification path — not the koi binary — is what is stuck in this session.
set -uo pipefail

OUT=${KOI_PROOF_DIR:-$HOME/.amp/in/artifacts/koi-proof}
CLS=org.omarchy.screensaver
export C="$CLS"
SHELL_JSON="$HOME/.config/omarchy/shell.json"
SHIM=/usr/share/omarchy/bin/omarchy-screensaver
LOG="$OUT/stock-idle-control.log"

cp "$SHELL_JSON" /tmp/koi-control-shell.json
restore_awake=0
cleanup() {
  sudo ln -sfn /usr/local/bin/koi-screensaver "$SHIM"
  cp /tmp/koi-control-shell.json "$SHELL_JSON"
  [[ $restore_awake == 1 ]] && omarchy-toggle-idle stay-awake >/dev/null 2>&1
}
trap cleanup EXIT

sudo ln -sfn /usr/bin/omarchy-screensaver "$SHIM"
if omarchy-toggle-idle status | grep -q '"enabled":true'; then
  omarchy-toggle-idle allow-idle >/dev/null
  restore_awake=1
fi
jq '.idle.screensaver = 1' "$SHELL_JSON" >/tmp/koi-control.json && mv /tmp/koi-control.json "$SHELL_JSON"

echo "== $(date -Is) shim -> $(readlink "$SHIM"); screensaver=1s; stay-awake off" | tee "$LOG"
wtype -M shift -m shift 2>/dev/null || true
for _ in $(seq 1 60); do
  n=$(hyprctl clients -j | jq '[.[] | select(.class == env.C)] | length')
  if ((n > 0)); then
    echo "stock screensaver appeared on idle" | tee -a "$LOG"
    sleep 1
    grim "$OUT/stock-idle.png"
    ps -eo args | grep -E '[t]tfx' | head -1 | tee -a "$LOG"
    wtype -k x 2>/dev/null || true
    exit 0
  fi
  sleep 0.5
done
echo "stock screensaver did NOT appear on idle either: $(omarchy-shell idle status | jq -c '{idle,lastEvent}')" | tee -a "$LOG"
exit 1
