use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq)]
pub enum WallpaperFill {
    Crop,
    Fit,
    Stretch,
    Center,
    Tile,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct HistoryEntry {
    pub path: String,
    pub timestamp: i64,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ScheduleEntry {
    pub time: String,
    pub path: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub enum IPCCommand {
    SetWallpaper {
        path: String,
        wayland_display: Option<String>,
        hyprland_instance: Option<String>,
        transition: Option<String>,
        duration: Option<u32>,
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
    GetHistory,
    RevertHistory,
    ScheduleSet {
        time: String,
        path: String,
    },
    GetSchedule,
    ToggleSchedule,
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
        history: Vec<HistoryEntry>,
        schedule_enabled: bool,
    },
    WallpaperList(Vec<String>),
    WallpaperDir(String),
    History(Vec<HistoryEntry>),
    Schedule {
        enabled: bool,
        schedule: Vec<ScheduleEntry>,
    },
}

pub fn get_socket_path() -> String {
    if let Ok(runtime_dir) = std::env::var("XDG_RUNTIME_DIR") {
        let path = std::path::PathBuf::from(runtime_dir).join("walllust.sock");
        return path.to_string_lossy().to_string();
    }
    "/tmp/walllust.sock".to_string()
}

#[allow(dead_code)]
pub fn check_dependencies() -> Vec<String> {
    let mut missing = Vec::new();
    let deps = ["ffmpeg", "mpvpaper", "wal"];
    for dep in deps {
        let found = std::process::Command::new("which")
            .arg(dep)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        if !found {
            missing.push(dep.to_string());
        }
    }
    missing
}
