#!/usr/bin/env bash
# Install (or remove) the koi pond as Omarchy's screensaver.
#
# Omarchy's idle service runs `omarchy-launch-screensaver`, which opens a terminal with
#   -e omarchy-screensaver
# resolved through PATH. On a running Omarchy session the first PATH entry is
# /usr/share/omarchy/bin, whose omarchy-* entries are symlinks into /usr/bin, so the shim there is
# the one the launcher actually runs. This script points that shim at the koi binary, leaves
# /usr/bin/omarchy-screensaver untouched, and installs a post-update hook so a package upgrade
# (which reinstalls the shim) puts the koi pond back.
set -euo pipefail

cd "$(dirname "$0")"
export PATH="$HOME/.cargo/bin:/usr/local/bin:$PATH"

BIN=/usr/local/bin/koi-screensaver
SHIM=/usr/local/bin/omarchy-screensaver
SESSION_SHIM=/usr/share/omarchy/bin/omarchy-screensaver
STATE=/usr/local/lib/koi-screensaver
STOCK=/usr/bin/omarchy-screensaver
HOOK="$HOME/.config/omarchy/hooks/post-update.d/koi-screensaver.hook"

as_root() {
  if [[ $EUID -eq 0 ]]; then "$@"; else sudo "$@"; fi
}

install_hook() {
  mkdir -p "$(dirname "$HOOK")"
  install -m755 "$(dirname "$0")/scripts/koi-screensaver.hook" "$HOOK"
}

case "${1:-install}" in
install)
  echo "==> building (release)"
  cargo build --release

  echo "==> installing $BIN"
  as_root install -Dm755 target/release/koi-screensaver "$BIN"
  as_root rm -f "$SHIM"
  as_root ln -s "$BIN" "$SHIM"

  echo "==> pointing the session shim at the koi pond"
  as_root mkdir -p "$STATE"
  if [[ ! -f "$STATE/stock-shim-target" ]]; then
    readlink -f "$SESSION_SHIM" | as_root tee "$STATE/stock-shim-target" >/dev/null
  fi
  as_root ln -sfn "$BIN" "$SESSION_SHIM"

  echo "==> installing the post-update hook"
  install_hook

  echo "==> checking resolution order"
  printf '    shim:              %s -> %s\n' "$SESSION_SHIM" "$(readlink "$SESSION_SHIM")"
  printf '    login shell:       %s\n' "$(command -v omarchy-screensaver || true)"
  printf '    version:           %s\n' "$("$BIN" --version)"

  cat <<EOF

Done. /usr/bin/omarchy-screensaver is untouched; this only redirects the shim the session runs.

  start it now      omarchy-launch-screensaver force
  exit it           press any key
  idle timeout      ~/.config/omarchy/shell.json -> idle.screensaver (seconds)
  disable           omarchy-toggle-enabled screensaver-off   (or set idle.screensaver to 0)
  uninstall         $0 uninstall

EOF
  ;;
uninstall)
  echo "==> restoring $SESSION_SHIM"
  target=$(cat "$STATE/stock-shim-target" 2>/dev/null || echo "$STOCK")
  as_root ln -sfn "$target" "$SESSION_SHIM"
  as_root rm -f "$SHIM" "$BIN"
  as_root rm -rf "$STATE"
  rm -f "$HOOK"
  echo "    shim now points at $(readlink "$SESSION_SHIM")"
  echo "    resolves to $(command -v omarchy-screensaver || echo nothing)"
  ;;
*)
  echo "usage: $0 [install|uninstall]" >&2
  exit 2
  ;;
esac
