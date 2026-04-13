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
        IPCResponse::Status { wallpaper, pywal, wallpapers_dir, .. } => {
            println!("Status:");
            println!("  Wallpaper: {:?}", wallpaper);
            println!("  Pywal: {}", pywal);
            println!("  Wallpaper Directory: {}", wallpapers_dir);
        },
        IPCResponse::WallpaperList(walls) => {
            println!("Wallpapers:");
            for wall in walls {
                println!("  {}", wall);
            }
        },
        IPCResponse::WallpaperDir(dir) => {
            println!("Wallpaper Directory: {}", dir);
        }
    }

    Ok(())
}
