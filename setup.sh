#!/usr/bin/env bash
# Download a temporary source checkout, then use the same installer as a local clone.
# Keep execution inside main so a truncated curl download cannot run a partial installer.
set -euo pipefail

main() {
  if [[ $# -gt 1 ]]; then
    echo "usage: bash setup.sh [install|uninstall]" >&2
    exit 2
  fi
  action=${1:-install}
  case "$action" in
    -h|--help)
      echo "usage: bash setup.sh [install|uninstall]"
      echo "Downloads the Rust koi screensaver and sets it up on Omarchy. Run without sudo."
      return
      ;;
    install|uninstall) ;;
    *) echo "Unknown action: $action (expected install or uninstall)" >&2; exit 2 ;;
  esac

  if [[ $EUID -eq 0 ]]; then
    echo "Run setup as your desktop user, without sudo. The installer requests sudo when needed." >&2
    exit 1
  fi
  for command in curl tar; do
    if ! command -v "$command" >/dev/null; then
      echo "Missing required command: $command" >&2
      exit 1
    fi
  done

  workdir=$(mktemp -d)
  trap 'rm -rf -- "$workdir"' EXIT
  trap 'exit 130' INT
  trap 'exit 143' TERM
  echo "==> downloading Omarchy Koi Screensaver"
  curl --fail --silent --show-error --location --proto '=https' \
    https://github.com/aarsh21/omarchy-koi-screensaver/archive/refs/heads/main.tar.gz \
    --output "$workdir/source.tar.gz"
  tar -xzf "$workdir/source.tar.gz" -C "$workdir" --strip-components=1
  bash "$workdir/install.sh" "$action"
}

main "$@"
