#!/usr/bin/env bash
# Final smoke test: launch through Omarchy's launcher and report what is on screen.
# Kept in a file because omarchy-launch-screensaver bails out when it sees the window class in
# another process's command line (`pgrep -f`), including the shell that started it.
set -euo pipefail
CLS=org.omarchy.screensaver
OUT=${KOI_PROOF_DIR:-$HOME/.amp/in/artifacts/koi-proof}
export C="$CLS"
mkdir -p "$OUT"

omarchy-launch-screensaver force >/dev/null 2>&1
sleep 5
windows=$(hyprctl clients -j | jq '[.[] | select(.class == env.C)]')
count=$(jq 'length' <<<"$windows")
[[ $count -gt 0 ]]
# The native canvas is a child of the launcher's waiting Foot host. ANSI has only the host.
pid=$(jq -r '(map(select(.title == "Koi pond")) + .)[0].pid' <<<"$windows")
if [[ $(readlink "/proc/$pid/exe") == /usr/local/bin/koi-screensaver ]]; then
  child=$pid
else
  child=$(pgrep -P "$pid" | head -1)
fi
printf 'windows=%s foot=%s child=%s exe=%s\n' "$count" "$pid" "$child" "$(readlink "/proc/$child/exe")"
jq -c '.[] | {class,title,pid,size,fullscreen,xwayland}' <<<"$windows"
[[ $(readlink "/proc/$child/exe") == /usr/local/bin/koi-screensaver ]]
ps -o pcpu=,pmem=,etime= -p "$child" | tr -s ' '
grim "$OUT/koi-final.png"
[[ $(hyprctl activewindow -j | jq -r .class) == "$CLS" ]]
wtype -k x
sleep 1.5
remaining=$(hyprctl clients -j | jq '[.[] | select(.class == env.C)] | length')
printf 'windows after key: %s\n' "$remaining"
[[ $remaining -eq 0 ]]
