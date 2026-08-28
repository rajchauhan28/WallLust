# WallLust

WallLust is a modern wallpaper daemon and GUI for Wayland and Hyprland. It features Pywal integration for dynamic color scheme generation, smooth transitions, and support for both image and video wallpapers.

## Features

- **Dynamic Transitions**: Smooth fade, slide, and grow transitions for image wallpapers.
- **Video Wallpaper Support**: Seamless integration with `mpvpaper`.
- **Interactive Scene Wallpapers**: Live, animated, cursor-reactive wallpapers written as `.slint` files.
- **Pywal Integration**: Automatically update your system colors when the wallpaper changes.
- **GUI & CLI**: Choose between a powerful CLI or a user-friendly GUI built with Slint.
- **IPC Daemon**: Lightweight daemon to manage wallpaper states across multiple outputs.

## Installation

Prebuilt packages for Arch, Debian and Fedora are attached to every
[release](https://github.com/rajchauhan28/WallLust/releases). Each one is built
inside that distribution's own container, so it links against the right glibc.

### Arch Linux

```bash
sudo pacman -U walllust-*-x86_64.pkg.tar.zst
```

### Debian / Ubuntu

Requires Debian 13 (trixie) or newer, or Ubuntu 24.04 or newer:
`walllust-renderer` needs `gtk4-layer-shell`, which Debian 12 does not
package, and GTK 4.12 or later.

```bash
sudo apt install ./walllust_*_amd64.deb
```

### Fedora

```bash
sudo dnf install ./walllust-*.x86_64.rpm
```

### From source

Build dependencies: a Rust toolchain, `clang`, `pkg-config`, and the
development headers for wayland, xkbcommon, EGL, fontconfig, GTK 4,
gtk4-layer-shell, WebKitGTK 6, Qt 6 and FFmpeg.

```bash
git clone https://github.com/rajchauhan28/WallLust
cd WallLust
./install.sh
```

`install.sh` installs into `~/.local`, and points the systemd user unit at
`~/.local/bin` instead of the `/usr/bin` path the packages use.

### Optional runtime tools

These are not required to start WallLust, but individual features need them:

| Tool | Needed for |
| --- | --- |
| `mpvpaper` | video wallpapers |
| `wal` (python-pywal) | colour scheme generation |
| `parec` (PipeWire/PulseAudio) | audio-reactive scene wallpapers |
| `hyprctl` (Hyprland) | cursor tracking in scene wallpapers |

### Building the packages yourself

```bash
./scripts/build-packages.sh all      # or: arch | deb | rpm
```

Requires Docker. Packages land in `dist/`.

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

# Set a video wallpaper (plays via mpvpaper)
walllust-cli set path/to/wallpaper.mp4

# Set an interactive scene wallpaper
walllust-cli set scenes/demo.slint

# List available wallpapers
walllust-cli list

# Get status
walllust-cli status
```

### Interactive Scene Wallpapers

A scene wallpaper is a [Slint](https://slint.dev) file exporting a component named `Main`.
It runs live on the desktop background: animations play continuously, and the wallpaper
can react to the mouse. See [`scenes/demo.slint`](scenes/demo.slint) for a complete example.

The daemon drives these properties if the scene declares them (all optional):

```slint
export component Main inherits Window {
    in-out property <float> time_ms;   // milliseconds since the scene started
    in-out property <float> time_s;    // seconds since the scene started
    in-out property <float> cursor_x;  // global cursor position in pixels
    in-out property <float> cursor_y;  // (Hyprland; updates even under windows)
    in-out property <float> audio_0;   // live audio spectrum band 0, range 0..1 (low -> high)
    // ... up to audio_7

}
```

Pointer events over exposed desktop areas are delivered natively, so `TouchArea`
hover/click interactions work directly. Each scene runs in its own renderer process,
so a broken scene can never take down the daemon.

### GUI

Simply run `walllust-gui` to open the wallpaper selector.

## License

MIT License. See [LICENSE](LICENSE) for details.
