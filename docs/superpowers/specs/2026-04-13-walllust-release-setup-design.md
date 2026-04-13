# WallLust Release Setup Design (2026-04-13)

Design specification for bug fixes, GitHub repository setup, and automated CI/CD releases for the WallLust project.

## 1. Bug Fixes & Refactoring

### Socket Path Resolution
Currently, the socket path is hardcoded to `/tmp/walllust.sock`. This can lead to permission issues if multiple users run the daemon on the same machine.
*   **Resolution**: Implement `get_socket_path()` in `src/common.rs`. It will prioritize `$XDG_RUNTIME_DIR/walllust.sock` (e.g., `/run/user/1000/walllust.sock`) and fall back to `/tmp/walllust.sock`. All components (daemon, CLI, GUI) will use this helper.

### Process & Dependency Management
*   **Process Tracking**: Update the daemon to use `tokio::process::Command` for spawning `mpvpaper` and `wal` to better track child processes and avoid zombies.
*   **Dependency Verification**: Add a utility `check_dependencies()` to verify if `ffmpeg`, `mpvpaper`, and `wal` are in the system's `PATH`. The daemon will log warnings if they are missing.
*   **Error Handling**: Replace `.unwrap()` calls in critical paths with `anyhow::Result` and proper error logging.

## 2. GitHub Setup

### Repository Details
*   **Repository Name**: `WallLust`
*   **Visibility**: Public
*   **Initial Commit**: Includes all source files, UI components, systemd service, and desktop entry.

### Documentation & Legal
*   **README.md**: Comprehensive documentation including:
    *   Description and features.
    *   Installation instructions (Debian/Arch/Source).
    *   Configuration and usage for CLI and GUI.
*   **License**: MIT License.

## 3. CI/CD & Packaging

### GitHub Action Workflow (`.github/workflows/release.yml`)
*   **Triggers**: On every tag push matching `v*` (e.g., `v0.1.0`).
*   **Build Job**:
    *   Builds release binaries for `x86_64-unknown-linux-gnu`.
    *   Uses `cargo-deb` to generate a `.deb` package.
    *   Creates a `.tar.gz` archive containing the binaries, `.desktop` file, and `.service` file.
*   **Release Job**:
    *   Creates a GitHub Release with the tag name.
    *   Uploads the `.deb` package and the `.tar.gz` archive as release assets.

### Arch Linux Packaging
*   **PKGBUILD**: Include a `PKGBUILD` in the repository to simplify AUR submissions and manual builds for Arch users.

## 4. Implementation Plan Summary
1.  **Phase 1: Bug Fixes**: Refactor socket path, improve process management, and add dependency checks.
2.  **Phase 2: Packaging Assets**: Add `cargo-deb` configuration and create the `PKGBUILD`.
3.  **Phase 3: GitHub Integration**: Create the repository, add the MIT license, `README.md`, and GitHub Action workflow.
4.  **Phase 4: Release**: Push the initial code and trigger the first release (v0.1.0).

---

## Spec Self-Review
*   **Placeholder scan**: None.
*   **Internal consistency**: All components use the same socket resolution logic.
*   **Scope check**: Covers the entire request (bug fixes + GitHub + Arch/Deb builds).
*   **Ambiguity check**: Clear resolution for socket path and process management.
