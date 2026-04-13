# WallLust

WallLust is a modern wallpaper daemon and GUI for Wayland and Hyprland. It features Pywal integration for dynamic color scheme generation, smooth transitions, and support for both image and video wallpapers.

## Features

- **Dynamic Transitions**: Smooth fade, slide, and grow transitions for image wallpapers.
- **Video Wallpaper Support**: Seamless integration with `mpvpaper`.
- **Pywal Integration**: Automatically update your system colors when the wallpaper changes.
- **GUI & CLI**: Choose between a powerful CLI or a user-friendly GUI built with Slint.
- **IPC Daemon**: Lightweight daemon to manage wallpaper states across multiple outputs.

## Installation

### Dependencies

Ensure you have the following installed:

- `slint`
- `mpvpaper` (for video wallpapers)
- `ffmpeg` (for video thumbnails)
- `python-pywal` (for color scheme generation)

### From Source

```bash
git clone https://github.com/rajchauhan28/WallLust
cd WallLust
cargo build --release
```

### Debian/Ubuntu

Download the `.deb` package from the [Releases](https://github.com/rajchauhan28/WallLust/releases) page and install it:

```bash
sudo dpkg -i walllust_*.deb
```

### Arch Linux

Use the provided `PKGBUILD` or install from AUR (once available).

## Usage

### Starting the Daemon

```bash
walllust-daemon &
```

Or enable the systemd user service:

```bash
systemctl --user enable --now walllust-daemon
```

### CLI Commands

```bash
# Set a wallpaper
walllust-cli set path/to/wallpaper.jpg --transition fade --duration 1000

# List available wallpapers
walllust-cli list

# Get status
walllust-cli status
```

### GUI

Simply run `walllust-gui` to open the wallpaper selector.

## License

MIT License. See [LICENSE](LICENSE) for details.
