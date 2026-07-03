#!/usr/bin/env bash
# rustcast installer — builds from source and sets everything up for the current user.
# Works on any Linux distro; the launcher itself runs on Wayland (Hyprland/Sway/
# GNOME/KDE) and X11.
set -euo pipefail

REPO_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BIN_DIR="${BIN_DIR:-$HOME/.local/bin}"
APP_DIR="$HOME/.local/share/applications"
CFG_DIR="$HOME/.config/rustcast"
SERVICE_DIR="$HOME/.config/systemd/user"

say()  { printf '\033[1;31m::\033[0m %s\n' "$1"; }
warn() { printf '\033[1;33m!!\033[0m %s\n' "$1"; }

# ── 1. dependency check ──────────────────────────────────────────────
if ! command -v cargo >/dev/null 2>&1; then
  warn "Rust (cargo) not found. Install it first:"
  echo "    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh"
  exit 1
fi

need_pkgs=""
pkgconf() { pkg-config --exists "$1" 2>/dev/null; }
if command -v pkg-config >/dev/null 2>&1; then
  pkgconf gtk4 || need_pkgs="$need_pkgs gtk4"
  pkgconf gtk4-layer-shell-0 || warn "gtk4-layer-shell not found — needed only for Hyprland/Sway overlay mode (GNOME/KDE fall back to a normal window)."
fi
if [ -n "$need_pkgs" ]; then
  warn "Missing build dependency:$need_pkgs. Install the dev packages, e.g.:"
  echo "    Arch:          sudo pacman -S gtk4 gtk4-layer-shell"
  echo "    Debian/Ubuntu: sudo apt install libgtk-4-dev libgtk4-layer-shell-dev"
  echo "    Fedora:        sudo dnf install gtk4-devel gtk4-layer-shell-devel"
  exit 1
fi

# ── 2. build ─────────────────────────────────────────────────────────
say "Building rustcast (release)…"
( cd "$REPO_DIR" && cargo build --release )

# ── 3. install binary ────────────────────────────────────────────────
say "Installing binary to $BIN_DIR"
mkdir -p "$BIN_DIR"
install -m755 "$REPO_DIR/target/release/rustcast" "$BIN_DIR/rustcast"
case ":$PATH:" in
  *":$BIN_DIR:"*) : ;;
  *) warn "$BIN_DIR is not on your PATH — add it in your shell rc." ;;
esac

# ── 4. desktop entry + config ────────────────────────────────────────
say "Installing desktop entry"
mkdir -p "$APP_DIR"
install -m644 "$REPO_DIR/packaging/rustcast.desktop" "$APP_DIR/rustcast.desktop"

mkdir -p "$CFG_DIR"
if [ ! -f "$CFG_DIR/config.toml" ]; then
  say "Writing default config to $CFG_DIR/config.toml"
  install -m644 "$REPO_DIR/config.example.toml" "$CFG_DIR/config.toml"
fi

# ── 5. clipboard daemon (systemd user service, optional) ─────────────
if command -v systemctl >/dev/null 2>&1 && command -v wl-paste >/dev/null 2>&1; then
  say "Enabling clipboard history daemon (systemd user service)"
  mkdir -p "$SERVICE_DIR"
  install -m644 "$REPO_DIR/packaging/rustcast-clipboard.service" "$SERVICE_DIR/rustcast-clipboard.service"
  systemctl --user daemon-reload || true
  systemctl --user enable --now rustcast-clipboard.service 2>/dev/null || \
    warn "Could not start the user service now (no graphical session?). It will start on next login."
else
  warn "Skipping clipboard daemon: needs systemd + wl-clipboard (Wayland). Copy/paste still works via xclip on X11."
fi

# ── 6. keybinding help ───────────────────────────────────────────────
cat <<EOF

$(say "Done. Bind a global hotkey to launch rustcast:")

  Hyprland  (~/.config/hypr/…):
      bind = SUPER, SPACE, exec, $BIN_DIR/rustcast
      bind = SUPER, V,     exec, $BIN_DIR/rustcast --tab clipboard

  Sway  (~/.config/sway/config):
      bindsym \$mod+space exec $BIN_DIR/rustcast
      bindsym \$mod+v     exec $BIN_DIR/rustcast --tab clipboard

  GNOME:  Settings → Keyboard → Custom Shortcuts → command:  rustcast
  KDE:    System Settings → Shortcuts → Custom → command:    rustcast

Run 'rustcast --help' for all flags.
EOF
