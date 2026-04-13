use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq)]
pub enum WallpaperFill {
    Crop,
    Fit,
    Stretch,
    Center,
    Tile,
}

#[derive(Debug, Serialize, Deserialize)]
pub enum IPCCommand {
    SetWallpaper {
        path: String,
        wayland_display: Option<String>,
        hyprland_instance: Option<String>,
        transition: Option<String>,
        duration: Option<u32>, // in milliseconds
    },
    ToggleDaemon,
    SetPywal(bool),
    GetStatus,
    ListWallpapers,
    SetFill(WallpaperFill),
    SetDefaultTransition {
        transition: String,
        duration: u32,
    },
    SetWallpaperDir(String),
    GetWallpapersDir,
    // Internal synchronization
    InternalFlip { new_is_1: bool, trans_dur: u64 },
    InternalRedraw { dur: u64, elapsed: u64, final_frame: bool },
}

#[derive(Debug, Serialize, Deserialize)]
pub enum IPCResponse {
    Success(String),
    Error(String),
    Status {
        wallpaper: Option<String>,
        pywal: bool,
        wallpapers_dir: String,
        default_transition: String,
        default_duration: u32,
    },
    WallpaperList(Vec<String>),
    WallpaperDir(String),
}

pub fn get_socket_path() -> String {
    if let Ok(runtime_dir) = std::env::var("XDG_RUNTIME_DIR") {
        let path = std::path::PathBuf::from(runtime_dir).join("walllust.sock");
        return path.to_string_lossy().to_string();
    }
    "/tmp/walllust.sock".to_string()
}
