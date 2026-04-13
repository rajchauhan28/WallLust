# WallLust Release Setup Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement bug fixes, refactor socket path, add dependency checks, and setup automated CI/CD for WallLust.

**Architecture:** Refactor hardcoded socket paths to use XDG standards, improve process management for mpvpaper/wal, and add a GitHub Action for automated .deb and binary releases.

**Tech Stack:** Rust (Tokio, Anyhow, Clap), Slint, GitHub Actions, cargo-deb.

---

### Task 1: Refactor Socket Path Resolution

**Files:**
- Modify: `src/common.rs`
- Modify: `src/daemon/main.rs`
- Modify: `src/cli/main.rs`
- Modify: `src/gui/main.rs`

- [ ] **Step 1: Add get_socket_path helper to src/common.rs**
```rust
pub fn get_socket_path() -> String {
    if let Ok(runtime_dir) = std::env::var("XDG_RUNTIME_DIR") {
        let path = std::path::PathBuf::from(runtime_dir).join("walllust.sock");
        return path.to_string_lossy().to_string();
    }
    "/tmp/walllust.sock".to_string()
}
```

- [ ] **Step 2: Update src/daemon/main.rs to use get_socket_path**
```rust
let socket_path = common::get_socket_path();
// ... instead of hardcoded "/tmp/walllust.sock"
```

- [ ] **Step 3: Update src/cli/main.rs to use get_socket_path**
```rust
let socket_path = common::get_socket_path();
```

- [ ] **Step 4: Update src/gui/main.rs to use get_socket_path**
```rust
let socket_path = common::get_socket_path();
```

- [ ] **Step 5: Verify build**
Run: `cargo check`
Expected: PASS

- [ ] **Step 6: Commit**
```bash
git add src/common.rs src/daemon/main.rs src/cli/main.rs src/gui/main.rs
git commit -m "refactor: use XDG_RUNTIME_DIR for IPC socket path"
```

### Task 2: Improve Process Management in Daemon

**Files:**
- Modify: `src/daemon/main.rs`

- [ ] **Step 1: Replace std::process::Command with tokio::process::Command for mpvpaper**
```rust
let mut cmd = tokio::process::Command::new("mpvpaper");
// ... set env vars ...
match cmd.spawn() {
    Ok(_) => println!("mpvpaper started successfully"),
    Err(e) => eprintln!("Failed to start mpvpaper: {}", e),
}
```

- [ ] **Step 2: Use tokio::process::Command for 'wal' and pkill calls**
```rust
let _ = tokio::process::Command::new("pkill").arg("-9").arg("mpvpaper").spawn();
let _ = tokio::process::Command::new("wal").args(&["-i", &path, "-n", "-e"]).spawn();
```

- [ ] **Step 3: Verify build**
Run: `cargo check`
Expected: PASS

- [ ] **Step 4: Commit**
```bash
git add src/daemon/main.rs
git commit -m "feat: use tokio::process for better process management"
```

### Task 3: Add Dependency Checks and Error Handling

**Files:**
- Modify: `src/common.rs`
- Modify: `src/daemon/main.rs`
- Modify: `Cargo.toml`

- [ ] **Step 1: Add anyhow dependency**
```toml
anyhow = "1.0"
```

- [ ] **Step 2: Add check_dependencies to src/common.rs**
```rust
pub fn check_dependencies() -> Vec<String> {
    let mut missing = Vec::new();
    let deps = ["ffmpeg", "mpvpaper", "wal"];
    for dep in deps {
        if std::process::Command::new("which").arg(dep).output().is_err() || 
           !std::process::Command::new("which").arg(dep).output().unwrap().status.success() {
            missing.push(dep.to_string());
        }
    }
    missing
}
```

- [ ] **Step 3: Call check_dependencies in daemon main.rs**
```rust
let missing = common::check_dependencies();
if !missing.is_empty() {
    eprintln!("Warning: Missing dependencies: {}", missing.join(", "));
}
```

- [ ] **Step 4: Commit**
```bash
git add Cargo.toml src/common.rs src/daemon/main.rs
git commit -m "feat: add dependency verification and anyhow support"
```

### Task 4: Add Packaging Configuration (cargo-deb, PKGBUILD)

**Files:**
- Modify: `Cargo.toml`
- Create: `PKGBUILD`

- [ ] **Step 1: Add [package.metadata.deb] to Cargo.toml**
```toml
[package.metadata.deb]
maintainer = "Raj Chauhan <raj@example.com>"
copyright = "2026, Raj Chauhan"
license-file = ["LICENSE", "0"]
depends = "slint-cpp, mpvpaper, ffmpeg, python3-pywal"
assets = [
    ["target/release/walllust-daemon", "usr/bin/", "755"],
    ["target/release/walllust-cli", "usr/bin/", "755"],
    ["target/release/walllust-gui", "usr/bin/", "755"],
    ["walllust-daemon.service", "usr/lib/systemd/user/", "644"],
    ["walllust.desktop", "usr/share/applications/", "644"],
]
```

- [ ] **Step 2: Create PKGBUILD for Arch Linux**
```bash
cat <<EOF > PKGBUILD
# Maintainer: Raj Chauhan <raj@example.com>
pkgname=walllust
pkgver=0.1.0
pkgrel=1
pkgdesc="A wallpaper daemon and GUI for Wayland/Hyprland with Pywal support"
arch=('x86_64')
url="https://github.com/rajchauhan28/WallLust"
license=('MIT')
depends=('slint' 'mpvpaper' 'ffmpeg' 'python-pywal')
makedepends=('cargo')
source=("\$pkgname-\$pkgver.tar.gz")
sha256sums=('SKIP')

build() {
    cd "\$pkgname-\$pkgver"
    cargo build --release --locked
}

package() {
    cd "\$pkgname-\$pkgver"
    install -Dm755 target/release/walllust-daemon "\$pkgdir/usr/bin/walllust-daemon"
    install -Dm755 target/release/walllust-cli "\$pkgdir/usr/bin/walllust-cli"
    install -Dm755 target/release/walllust-gui "\$pkgdir/usr/bin/walllust-gui"
    install -Dm644 walllust-daemon.service "\$pkgdir/usr/lib/systemd/user/walllust-daemon.service"
    install -Dm644 walllust.desktop "\$pkgdir/usr/share/applications/walllust.desktop"
}
EOF
```

- [ ] **Step 3: Commit**
```bash
git add Cargo.toml PKGBUILD
git commit -m "chore: add packaging config for Deb and Arch"
```

### Task 5: Add Documentation, License, and GitHub Action

**Files:**
- Create: `README.md`
- Create: `LICENSE`
- Create: `.github/workflows/release.yml`

- [ ] **Step 1: Create MIT License file**

- [ ] **Step 2: Create README.md with installation instructions**

- [ ] **Step 3: Create GitHub Action for releases**
```yaml
name: Release
on:
  push:
    tags: ["v*"]
jobs:
  release:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - run: cargo install cargo-deb
      - run: cargo build --release
      - run: cargo deb
      - name: Release
        uses: softprops/action-gh-release@v2
        with:
          files: |
            target/debian/*.deb
            target/release/walllust-daemon
            target/release/walllust-cli
            target/release/walllust-gui
```

- [ ] **Step 4: Commit**
```bash
git add README.md LICENSE .github/workflows/release.yml
git commit -m "docs: add readme, license, and release workflow"
```

### Task 6: Repository Creation and Initial Push

- [ ] **Step 1: Create GitHub Repo**
Run: `gh repo create WallLust --public --source=. --remote=origin --push`

- [ ] **Step 2: Tag v0.1.0**
Run: `git tag v0.1.0 && git push origin v0.1.0`

- [ ] **Step 3: Verify GitHub Action triggers**
Check the repository's 'Actions' tab.
