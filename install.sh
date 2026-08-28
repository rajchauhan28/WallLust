#!/bin/bash

# Walllust Install Script
# This script builds the project, installs binaries, and sets up a systemd user service.

set -e

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

echo -e "${BLUE}Starting Walllust installation...${NC}"

BUILD_MODE="release"
CARGO_CMD="cargo build --release"
TARGET_DIR="target/release"

if [ "$1" == "--dev" ]; then
    BUILD_MODE="debug"
    CARGO_CMD="cargo build"
    TARGET_DIR="target/debug"
    echo -e "${BLUE}Running in DEV mode. Will use unoptimized debug build to save compilation time.${NC}"
fi

# Check for dependencies
if ! command -v cargo &> /dev/null; then
    echo -e "${RED}Error: cargo is not installed. Please install Rust and Cargo first.${NC}"
    exit 1
fi

if ! command -v mpvpaper &> /dev/null; then
    echo -e "${RED}Warning: mpvpaper is not installed. Video wallpapers will not work without it.${NC}"
fi

# 1. Build the project
# Check if we are on an Arch-based system and install new WebKit renderer dependencies
if command -v pacman &> /dev/null; then
    echo "Installing required WebKit dependencies (webkitgtk-6.0, gtk4-layer-shell)..."
    sudo pacman -S --needed --noconfirm webkitgtk-6.0 gtk4-layer-shell || true
fi

echo -e "${BLUE}Building project in $BUILD_MODE mode...${NC}"
$CARGO_CMD

# 2. Stop the service if it exists to avoid 'Text file busy'
if systemctl --user is-active --quiet walllust-daemon.service; then
    echo -e "${BLUE}Stopping running walllust-daemon service...${NC}"
    systemctl --user stop walllust-daemon.service
fi

# 3. Create local bin directory if it doesn't exist
INSTALL_DIR="$HOME/.local/bin"
mkdir -p "$INSTALL_DIR"

# 4. Install binaries
echo -e "${BLUE}Installing binaries to $INSTALL_DIR...${NC}"
killall walllust-gui || true
rm -f "$INSTALL_DIR/walllust-daemon" "$INSTALL_DIR/walllust-cli" "$INSTALL_DIR/walllust-gui"
cp $TARGET_DIR/walllust-daemon "$INSTALL_DIR/"
cp $TARGET_DIR/walllust-cli "$INSTALL_DIR/"
cp $TARGET_DIR/walllust-gui "$INSTALL_DIR/"

# 5. Set up systemd user service
echo -e "${BLUE}Setting up systemd user service...${NC}"
SERVICE_DIR="$HOME/.config/systemd/user"
mkdir -p "$SERVICE_DIR"
rm -f "$SERVICE_DIR/walllust-daemon.service"
# The committed unit points at /usr/bin so the distribution packages
# work. A source install puts the binary in ~/.local/bin instead.
sed 's|^ExecStart=/usr/bin/walllust-daemon|ExecStart=%h/.local/bin/walllust-daemon|' \
    walllust-daemon.service > "$SERVICE_DIR/walllust-daemon.service"
chmod 644 "$SERVICE_DIR/walllust-daemon.service"

# 6. Set up desktop entry
echo -e "${BLUE}Setting up desktop entry...${NC}"
DESKTOP_DIR="$HOME/.local/share/applications"
mkdir -p "$DESKTOP_DIR"
rm -f "$DESKTOP_DIR/walllust.desktop"
# Replace %h with actual home path in the desktop file during copy
sed "s|%h|$HOME|g" walllust.desktop > "$DESKTOP_DIR/walllust.desktop"

# 7. Reload systemd, enable and start the service
echo -e "${BLUE}Enabling and starting walllust-daemon service...${NC}"
systemctl --user daemon-reload
systemctl --user enable walllust-daemon.service
systemctl --user restart walllust-daemon.service

echo -e "${GREEN}Installation complete!${NC}"
echo -e "Binaries installed to: $INSTALL_DIR"
echo -e "You can now run 'walllust-gui' to start the wallpaper manager."
echo -e "The daemon is running in the background as a systemd user service."
echo -e "Check status with: systemctl --user status walllust-daemon"

echo -e "\n${BLUE}Note for Wayland users:${NC}"
echo -e "Ensure your compositor imports its environment to systemd so the daemon can find the display."
echo -e "Example for Hyprland: add 'exec-once = dbus-update-activation-environment --systemd WAYLAND_DISPLAY XDG_CURRENT_DESKTOP' to your config."
echo -e "You can also run 'systemctl --user import-environment WAYLAND_DISPLAY' manually."
