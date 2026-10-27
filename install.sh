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

ROOT=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
cd "$ROOT"
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

if [[ $# -gt 1 ]]; then
  echo "usage: $0 [install|uninstall]" >&2
  exit 2
fi

case "${1:-install}" in
install)
  if [[ "$(uname -s)" != Linux ]] || ! command -v pacman >/dev/null ||
    ! command -v omarchy-launch-screensaver >/dev/null; then
    echo "This installer requires Omarchy on Arch Linux." >&2
    exit 1
  fi
  if [[ ! -L $SESSION_SHIM || ! -x $STOCK ]]; then
    echo "Unsupported Omarchy layout: expected $SESSION_SHIM to be a symlink and $STOCK to exist." >&2
    exit 1
  fi
  if [[ ( -e $SHIM || -L $SHIM ) && $(readlink "$SHIM" || true) != "$BIN" ]]; then
    echo "Refusing to replace another screensaver at $SHIM. Move it aside before installing." >&2
    exit 1
  fi

  packages=()
  if ! command -v cc >/dev/null || ! command -v make >/dev/null; then
    packages+=(base-devel)
  fi
  if ! command -v pkg-config >/dev/null; then
    packages+=(pkgconf)
  fi
  if ! command -v pkg-config >/dev/null || ! pkg-config --exists sdl3; then
    packages+=(sdl3)
  fi
  if ! command -v pkg-config >/dev/null || ! pkg-config --exists cairo; then
    packages+=(cairo)
  fi
  if ! cargo --version >/dev/null 2>&1 || ! rustc --version >/dev/null 2>&1; then
    if command -v rustup >/dev/null; then
      echo "==> installing a stable Rust toolchain for this build"
      rustup toolchain install stable --profile minimal
      export RUSTUP_TOOLCHAIN=stable
    else
      packages+=(rust)
    fi
  fi
  if [[ ${#packages[@]} -gt 0 ]]; then
    echo "==> installing missing build dependencies: ${packages[*]}"
    as_root pacman -S --needed --noconfirm "${packages[@]}"
  fi

  echo "==> building (release)"
  cargo build --locked --release --target-dir "$ROOT/target"

  echo "==> preserving the previous screensaver and an offline uninstaller"
  as_root mkdir -p "$STATE"
  if [[ ! -f "$STATE/stock-shim-target" ]]; then
    target=$(readlink "$SESSION_SHIM")
    [[ $target != "$BIN" ]] || target=$STOCK
    printf '%s\n' "$target" | as_root tee "$STATE/stock-shim-target" >/dev/null
  fi
  as_root install -m755 "$ROOT/install.sh" "$STATE/install.sh"

  echo "==> installing $BIN"
  as_root install -Dm755 target/release/koi-screensaver "$BIN"
  as_root ln -sfn "$BIN" "$SHIM"
  as_root ln -sfn "$BIN" "$SESSION_SHIM"

  echo "==> installing the post-update hook"
  mkdir -p "$(dirname "$HOOK")"
  install -m755 "$ROOT/scripts/koi-screensaver.hook" "$HOOK"

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
  uninstall         bash $STATE/install.sh uninstall

EOF
  ;;
uninstall)
  if [[ ! -f "$STATE/stock-shim-target" ]]; then
    echo "No saved koi installation found. Nothing changed."
    exit 0
  fi
  if [[ $(readlink "$SESSION_SHIM" || true) == "$BIN" ]]; then
    echo "==> restoring $SESSION_SHIM"
    target=$(cat "$STATE/stock-shim-target")
    as_root ln -sfn "$target" "$SESSION_SHIM"
  else
    echo "==> session shim was changed after installation; leaving it untouched"
  fi
  if [[ $(readlink "$SHIM" || true) == "$BIN" ]]; then
    as_root rm -f "$SHIM"
  fi
  as_root rm -f "$BIN"
  as_root rm -rf "$STATE"
  rm -f "$HOOK"
  echo "    shim now points at $(readlink "$SESSION_SHIM")"
  echo "    resolves to $(command -v omarchy-screensaver || echo nothing)"
  ;;
-h|--help)
  echo "usage: $0 [install|uninstall]"
  ;;
*)
  echo "usage: $0 [install|uninstall]" >&2
  exit 2
  ;;
esac
