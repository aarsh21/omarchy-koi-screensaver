#!/usr/bin/env bash
# Final smoke test: launch through Omarchy's launcher and report what is on screen.
set -euo pipefail
CLS=org.omarchy.screensaver
OUT=${KOI_PROOF_DIR:-$HOME/.amp/in/artifacts/koi-proof}
export C="$CLS"
mkdir -p "$OUT"

wait_for_count() {
  local attempt
  for attempt in {1..100}; do
    [[ $(hyprctl clients -j | jq '[.[] | select(.class == env.C)] | length') -eq $1 ]] && return 0
    sleep 0.05
  done
  return 1
}

omarchy-launch-screensaver force >/dev/null 2>&1
sleep 5
windows=$(hyprctl clients -j | jq '[.[] | select(.class == env.C)]')
count=$(jq 'length' <<<"$windows")
ponds=$(jq '[.[] | select(.title == "Koi pond")]' <<<"$windows")
expected=$(hyprctl monitors -j | jq length)
[[ $count -eq $expected ]]
[[ $(jq length <<<"$ponds") -eq $expected ]]
[[ $(jq '[.[].monitor] | unique | length' <<<"$ponds") -eq $expected ]]
while read -r native_pid; do
  [[ $(readlink "/proc/$native_pid/exe") == /usr/local/bin/koi-screensaver ]]
done < <(jq -r '.[].pid' <<<"$ponds")
# Repeated requests while active must not add windows.
for attempt in {1..4}; do omarchy-launch-screensaver force >/dev/null 2>&1 & done
wait
sleep 0.2
[[ $(hyprctl clients -j | jq '[.[] | select(.class == env.C)] | length') -eq $expected ]]
printf 'native ponds=%s; one per output\n' "$expected"
pid=$(jq -r '.[0].pid' <<<"$ponds")
printf 'windows=%s native_pid=%s exe=%s; no terminal host\n' "$count" "$pid" "$(readlink "/proc/$pid/exe")"
jq -c '.[] | {class,title,pid,size,fullscreen,xwayland}' <<<"$windows"
ps -o pcpu=,pmem=,etime= -p "$pid" | tr -s ' '
grim "$OUT/koi-final.png"
while read -r monitor; do
  hyprctl dispatch "hl.dsp.focus({ monitor = \"$monitor\" })" >/dev/null 2>&1 ||
    hyprctl dispatch focusmonitor "$monitor" >/dev/null
  sleep 0.5
  [[ $(hyprctl clients -j | jq '[.[] | select(.class == env.C and .title == "Koi pond")] | length') -eq $expected ]]
done < <(hyprctl monitors -j | jq -r '.[].name')
[[ $(hyprctl activewindow -j | jq -r .class) == "$CLS" ]]
wtype -k x
wait_for_count 0
remaining=$(hyprctl clients -j | jq '[.[] | select(.class == env.C)] | length')
printf 'windows after key: %s\n' "$remaining"
[[ $remaining -eq 0 ]]

# Reopen as soon as the surface disappears, before the old supervisor necessarily exits.
for attempt in {1..3}; do
  omarchy-launch-screensaver force >/dev/null 2>&1
  wait_for_count "$expected"
  [[ $(hyprctl activewindow -j | jq -r .class) == "$CLS" ]]
  wtype -k x
  wait_for_count 0
done
printf 'rapid reopen/dismiss cycles: 3; duplicate active requests: 4; no extra windows\n'
