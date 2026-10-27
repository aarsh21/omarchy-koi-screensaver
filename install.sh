#!/usr/bin/env bash
# Install (or remove) the koi pond as Omarchy's screensaver.
#
# Redirect Omarchy's session/login-shell shims to the Rust launcher and pond. The native launcher
# opens one pond per output without a waiting terminal window. Stock executables stay untouched.
set -euo pipefail

ROOT=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
cd "$ROOT"
export PATH="$HOME/.cargo/bin:/usr/local/bin:$PATH"

BIN=/usr/local/bin/koi-screensaver
SHIM=/usr/local/bin/omarchy-screensaver
SESSION_SHIM=/usr/share/omarchy/bin/omarchy-screensaver
LAUNCHER=/usr/local/bin/omarchy-launch-screensaver
SESSION_LAUNCHER=/usr/share/omarchy/bin/omarchy-launch-screensaver
STATE=/usr/local/lib/koi-screensaver
STOCK=/usr/bin/omarchy-screensaver
STOCK_LAUNCHER=/usr/bin/omarchy-launch-screensaver
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
  if [[ ! -L $SESSION_SHIM || ! -x $STOCK || ! -L $SESSION_LAUNCHER || ! -x $STOCK_LAUNCHER ]]; then
    echo "Unsupported Omarchy layout: screensaver and launcher session shims must be symlinks to stock executables." >&2
    exit 1
  fi
  for shim in "$SHIM" "$LAUNCHER"; do
    if [[ ( -e $shim || -L $shim ) && $(readlink "$shim" || true) != "$BIN" ]]; then
      echo "Refusing to replace another screensaver at $shim. Move it aside before installing." >&2
      exit 1
    fi
  done

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
  if [[ ! -f "$STATE/stock-launcher-target" ]]; then
    target=$(readlink "$SESSION_LAUNCHER")
    [[ $target != "$BIN" ]] || target=$STOCK_LAUNCHER
    printf '%s\n' "$target" | as_root tee "$STATE/stock-launcher-target" >/dev/null
  fi
  as_root install -m755 "$ROOT/install.sh" "$STATE/install.sh"

  echo "==> installing $BIN"
  as_root install -Dm755 target/release/koi-screensaver "$BIN"
  for shim in "$SHIM" "$SESSION_SHIM" "$LAUNCHER" "$SESSION_LAUNCHER"; do
    as_root ln -sfn "$BIN" "$shim"
  done

  echo "==> installing the post-update hook"
  mkdir -p "$(dirname "$HOOK")"
  install -m755 "$ROOT/scripts/koi-screensaver.hook" "$HOOK"

  echo "==> checking resolution order"
  printf '    shim:              %s -> %s\n' "$SESSION_SHIM" "$(readlink "$SESSION_SHIM")"
  printf '    login shell:       %s\n' "$(command -v omarchy-screensaver || true)"
  printf '    native launcher:   %s -> %s\n' "$SESSION_LAUNCHER" "$(readlink "$SESSION_LAUNCHER")"
  printf '    version:           %s\n' "$("$BIN" --version)"

  cat <<EOF

Done. Stock /usr/bin executables are untouched; the session shims now use the native pond.

  start it now      omarchy-launch-screensaver force
  exit it           press any key
  idle timeout      ~/.config/omarchy/shell.json -> idle.screensaver (seconds)
  disable           omarchy-toggle-screensaver   (or set idle.screensaver to 0)
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
  if [[ -f "$STATE/stock-launcher-target" && $(readlink "$SESSION_LAUNCHER" || true) == "$BIN" ]]; then
    as_root ln -sfn "$(cat "$STATE/stock-launcher-target")" "$SESSION_LAUNCHER"
  fi
  if [[ $(readlink "$LAUNCHER" || true) == "$BIN" ]]; then
    as_root rm -f "$LAUNCHER"
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
