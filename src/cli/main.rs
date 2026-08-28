use clap::{Parser, Subcommand};
use tokio::net::UnixStream;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use serde_json;

#[path = "../common.rs"]
mod common;
use common::{IPCCommand, IPCResponse};

#[derive(Parser)]
#[command(name = "walllust-cli")]
#[command(about = "Wallpaper changer CLI", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Set a wallpaper
    Set { 
        path: String,
        /// Transition type (fade, slide, grow)
        #[arg(short, long)]
        transition: Option<String>,
        /// Transition duration in milliseconds
        #[arg(short, long)]
        duration: Option<u32>,
    },
    /// Toggle Pywal compatibility
    Pywal { 
        #[arg(short, long)]
        off: bool 
    },
    /// List available wallpapers
    List,
    /// Get daemon status
    Status,
    /// Set wallpaper directory
    Dir { path: String },
    /// Show wallpaper history (recently used)
    History,
    /// Revert to previous wallpaper
    Revert,
    /// Add a scheduled wallpaper change (format: "HH:MM")
    Schedule { 
        #[arg(short, long)]
        time: Option<String>,
        path: Option<String>,
    },
    /// Toggle schedule on/off
    ToggleSchedule,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();
    let socket_path = common::get_socket_path();

    let mut stream = UnixStream::connect(socket_path).await?;
    
    let ipc_cmd = match cli.command {
        Commands::Set { path, transition, duration } => {
            let wayland_display = std::env::var("WAYLAND_DISPLAY").ok();
            let hyprland_instance = std::env::var("HYPRLAND_INSTANCE_SIGNATURE").ok();
            IPCCommand::SetWallpaper {
                path,
                wayland_display,
                hyprland_instance,
                transition,
                duration,
            }
        },
        Commands::Pywal { off } => IPCCommand::SetPywal(!off),
        Commands::List => IPCCommand::ListWallpapers,
        Commands::Status => IPCCommand::GetStatus,
        Commands::Dir { path } => IPCCommand::SetWallpaperDir(path),
        Commands::History => IPCCommand::GetHistory,
        Commands::Revert => IPCCommand::RevertHistory,
        Commands::Schedule { time, path } => {
            if let (Some(t), Some(p)) = (time, path) {
                IPCCommand::ScheduleSet { time: t, path: p }
            } else {
                eprintln!("Error: --time and path are required for schedule");
                std::process::exit(1);
            }
        },
        Commands::ToggleSchedule => IPCCommand::ToggleSchedule,
    };

    let cmd_bytes = serde_json::to_vec(&ipc_cmd)?;
    stream.write_all(&cmd_bytes).await?;
    stream.shutdown().await?; 

    let mut buffer = Vec::new();
    stream.read_to_end(&mut buffer).await?;
    
    let response: IPCResponse = serde_json::from_slice(&buffer)?;
    match response {
        IPCResponse::Success(msg) => println!("Success: {}", msg),
        IPCResponse::Error(msg) => println!("Error: {}", msg),
        IPCResponse::Status { wallpaper, pywal, wallpapers_dir, default_transition, default_duration, .. } => {
            println!("Status:");
            println!("  Wallpaper: {:?}", wallpaper);
            println!("  Pywal: {}", pywal);
            println!("  Wallpaper Directory: {}", wallpapers_dir);
            println!("  Default Transition: {} ({}ms)", default_transition, default_duration);
        },
        IPCResponse::WallpaperList(walls) => {
            println!("Wallpapers:");
            for wall in walls {
                println!("  {}", wall);
            }
        },
        IPCResponse::WallpaperDir(dir) => {
            println!("Wallpaper Directory: {}", dir);
        },
        IPCResponse::History(entries) => {
            if entries.is_empty() {
                println!("No wallpaper history found.");
            } else {
                println!("Wallpaper History ({} entries):", entries.len());
                for (i, entry) in entries.iter().enumerate().take(10) {
                    let ts = std::time::UNIX_EPOCH + std::time::Duration::from_secs(entry.timestamp as u64);
                    println!("  {}. {} ({:?})", i + 1, entry.path, ts);
                }
            }
        },
        IPCResponse::Schedule { enabled, schedule } => {
            println!("Schedule: {}", if enabled { "enabled" } else { "disabled" });
            if schedule.is_empty() {
                println!("  No scheduled wallpapers.");
            } else {
                println!("  Scheduled entries:");
                for entry in schedule {
                    println!("    {}: {}", entry.time, entry.path);
                }
            }
        },
    }

    Ok(())
}
