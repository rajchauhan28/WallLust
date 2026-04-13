use tokio::net::{UnixListener, UnixStream};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use std::path::{Path, PathBuf};
use layer_shika::prelude::*;
use layer_shika::slint::ComponentHandle;
use clap::Parser;

#[path = "../common.rs"]
mod common;
use common::{IPCCommand, IPCResponse};

#[derive(Parser)]
#[command(name = "walllust-daemon")]
#[command(about = "Wallpaper daemon for walllust", long_about = None)]
struct Args {
    /// Socket path to listen on
    #[arg(short, long, default_value = "/tmp/walllust.sock")]
    socket: String,

    /// Wallpapers directory
    #[arg(short, long)]
    wallpapers_dir: Option<PathBuf>,

    /// Disable Pywal integration
    #[arg(long)]
    no_pywal: bool,

    /// Default transition type (fade, slide, grow)
    #[arg(short, long, default_value = "fade")]
    transition: String,

    /// Default transition duration in milliseconds
    #[arg(short, long, default_value_t = 1000)]
    duration: u32,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct DaemonState {
    current_wallpaper: Option<String>,
    pywal_enabled: bool,
    wallpapers_dir: PathBuf,
    default_transition: String,
    default_duration: u32,
}

impl DaemonState {
    fn save(&self) {
        if let Ok(json) = serde_json::to_string_pretty(self) {
            let config_dir = dirs::home_dir().unwrap().join(".config/walllust");
            let _ = std::fs::create_dir_all(&config_dir);
            let _ = std::fs::write(config_dir.join("config.json"), json);
        }
    }

    fn load(args: &Args) -> Self {
        let config_path = dirs::home_dir().unwrap().join(".config/walllust/config.json");
        if let Ok(content) = std::fs::read_to_string(&config_path) {
            if let Ok(state) = serde_json::from_str::<DaemonState>(&content) {
                return state;
            }
        }
        
        let wallpapers_dir = args.wallpapers_dir.clone().unwrap_or_else(|| {
            dirs::home_dir().unwrap().join("Pictures/wallpapers")
        });

        DaemonState {
            current_wallpaper: None,
            pywal_enabled: !args.no_pywal,
            wallpapers_dir,
            default_transition: args.transition.clone(),
            default_duration: args.duration,
        }
    }
}

#[tokio::main]
async fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();

    let missing = common::check_dependencies();
    if !missing.is_empty() {
        eprintln!("Warning: Missing dependencies: {}", missing.join(", "));
    }

    let socket_path = common::get_socket_path();

    if Path::new(&socket_path).exists() {
        let _ = std::fs::remove_file(&socket_path);
    }

    let listener = UnixListener::bind(&socket_path)?;
    println!("Daemon listening on {}", socket_path);

    let state = std::sync::Arc::new(tokio::sync::Mutex::new(DaemonState::load(&args)));

    if !state.lock().await.wallpapers_dir.exists() {
        let _ = std::fs::create_dir_all(&state.lock().await.wallpapers_dir);
    }

    let (slint_tx_internal, mut slint_rx_internal) = tokio::sync::mpsc::channel::<IPCCommand>(10);

    // Start Slint/Layer-Shell in its own thread
    let slint_tx_for_thread = slint_tx_internal.clone();
    std::thread::spawn(move || {
        let ui_source = include_str!("../../ui/wallpaper.slint");
        
        println!("Initializing Layer Shell...");
        let mut shell = loop {
            match Shell::from_source(ui_source)
                .surface("Wallpaper")
                .layer(Layer::Bottom)
                .anchor(AnchorEdges::all())
                .output_policy(OutputPolicy::AllOutputs)
                .build() 
            {
                Ok(s) => break s,
                Err(e) => {
                    eprintln!("Failed to build layer-shika shell: {}. Retrying in 2 seconds...", e);
                    std::thread::sleep(std::time::Duration::from_secs(2));
                }
            }
        };

        println!("Registering IPC channel in Slint event loop...");
        let tx_internal_for_closure = slint_tx_for_thread.clone();
        let (_, tx_layer_shika) = shell.event_loop_handle().add_channel::<IPCCommand, _>(move |cmd, app_state| {
                if let IPCCommand::SetWallpaper { path, wayland_display, hyprland_instance, transition, duration } = cmd {
                let path_buf = PathBuf::from(&path);
                let absolute_path = path_buf.canonicalize().unwrap_or_else(|_| path_buf.clone());
                let ext = absolute_path.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
                let path_str = absolute_path.to_string_lossy().to_string();

                let video_extensions = ["mp4", "mkv", "webm", "mov", "avi", "flv", "wmv", "mpg", "mpeg", "gif"];
                if video_extensions.contains(&ext.as_str()) {
                    println!("Video detected: {}. Hiding native surface and launching mpvpaper...", path_str);
                    for surface in app_state.surfaces_by_name("Wallpaper") {
                        let component = surface.component_instance();
                        let _ = component.set_property("surface_visible", slint_interpreter::Value::Bool(false));
                    }
                    handle_video_wallpaper(&path_str, wayland_display, hyprland_instance);
                    return;
                }

                println!("Image detected: {}. Killing mpvpaper and showing native surface...", path_str);
                match slint::Image::load_from_path(&absolute_path) {
                    Ok(img) => {
                        let trans_type = transition.unwrap_or_else(|| "fade".to_string());
                        let trans_dur = duration.unwrap_or(1000) as f64;

                        // Kill mpvpaper
                        let _ = tokio::process::Command::new("pkill").arg("-9").arg("mpvpaper").spawn();

                        for surface in app_state.surfaces_by_name("Wallpaper") {
                            let component = surface.component_instance();
                            let is_1 = match component.get_property("active_is_1") {
                                Ok(slint_interpreter::Value::Bool(b)) => b,
                                _ => false,
                            };
                            let new_is_1 = !is_1;
                            let img_prop = if new_is_1 { "image1" } else { "image2" };

                            let _ = component.set_property("surface_visible", slint_interpreter::Value::Bool(true));
                            let _ = component.set_property(img_prop, slint_interpreter::Value::Image(img.clone()));
                            let _ = component.set_property("transition_type", slint_interpreter::Value::from(slint::SharedString::from(&trans_type)));
                            let _ = component.set_property("transition_duration_ms", slint_interpreter::Value::Number(trans_dur));
                            
                            let tx = tx_internal_for_closure.clone();
                            let trans_dur_u64 = trans_dur as u64;
                            
                            // Spawn thread to push a flip safely on the exact native loop without invoke_from_event_loop failures
                            std::thread::spawn(move || {
                                std::thread::sleep(std::time::Duration::from_millis(2)); // Reduced delay to 2ms down from 32ms
                                let _ = tx.blocking_send(IPCCommand::InternalFlip { new_is_1, trans_dur: trans_dur_u64 });
                            });
                        }
                    },
                    Err(e) => eprintln!("Error loading image {}: {}", path, e),
                }
            } else if let IPCCommand::InternalFlip { new_is_1, trans_dur } = cmd {
                // FLIP HAS ARRIVED ON THE CORRECT THREAD
                for surface in app_state.surfaces_by_name("Wallpaper") {
                    let component = surface.component_instance();
                    let _ = slint_interpreter::ComponentInstance::set_property(&component, "active_is_1", slint_interpreter::Value::Bool(new_is_1));
                }
                
                let tx = tx_internal_for_closure.clone();
                std::thread::spawn(move || {
                    let start = std::time::Instant::now();
                    loop {
                        let elapsed = start.elapsed().as_millis() as u64;
                        if elapsed >= trans_dur {
                            let _ = tx.blocking_send(IPCCommand::InternalRedraw { dur: trans_dur, elapsed, final_frame: true });
                            break;
                        }
                        let _ = tx.blocking_send(IPCCommand::InternalRedraw { dur: trans_dur, elapsed, final_frame: false });
                        std::thread::sleep(std::time::Duration::from_millis(16));
                    }
                });
            } else if let IPCCommand::InternalRedraw { dur, elapsed, final_frame } = cmd {
                if final_frame {
                    println!("Transition progress: 100.00%");
                } else {
                    let pct = (elapsed as f64 / dur as f64) * 100.0;
                    println!("Transition progress: {:.2}%", pct);
                }
                
                for surface in app_state.surfaces_by_name("Wallpaper") {
                    let component = surface.component_instance();
                    component.window().request_redraw();
                }
            }
        }).expect("Failed to add channel to event loop");

        let tx_bridge = tx_layer_shika.clone();
        std::thread::spawn(move || {
            while let Some(cmd) = slint_rx_internal.blocking_recv() {
                if let Err(e) = tx_bridge.send(cmd) {
                    eprintln!("Bridge error: {}", e);
                }
            }
        });

        println!("Starting Layer Shell event loop...");
        shell.run().expect("Failed to run layer-shika shell");
    });

    loop {
        match listener.accept().await {
            Ok((stream, _)) => {
                let s = state.clone();
                let sender = slint_tx_internal.clone();
                tokio::spawn(async move {
                    handle_client(stream, s, sender).await;
                });
            }
            Err(e) => println!("Error accepting: {}", e),
        }
    }
}

fn handle_video_wallpaper(path: &str, wayland_display: Option<String>, hyprland_instance: Option<String>) {
    println!("Killing existing mpvpaper processes...");
    let _ = tokio::process::Command::new("pkill").arg("-9").arg("mpvpaper").spawn();
    
    let mut cmd = tokio::process::Command::new("mpvpaper");
    
    if let Some(wd) = wayland_display {
        println!("Setting WAYLAND_DISPLAY={}", wd);
        cmd.env("WAYLAND_DISPLAY", wd);
    }
    if let Some(hi) = hyprland_instance {
        println!("Setting HYPRLAND_INSTANCE_SIGNATURE={}", hi);
        cmd.env("HYPRLAND_INSTANCE_SIGNATURE", hi);
    }

    // Explicitly set some common env vars just in case
    cmd.env("XDG_RUNTIME_DIR", std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| format!("/run/user/{}", unsafe { libc::getuid() })));
    
    // Ensure PATH includes common locations
    let current_path = std::env::var("PATH").unwrap_or_default();
    let home = dirs::home_dir().unwrap();
    let local_bin = home.join(".local/bin");
    let new_path = format!("{}:{}:/usr/local/bin:/usr/bin:/bin", current_path, local_bin.display());
    cmd.env("PATH", new_path);

    cmd.args(&[
        "-f", 
        "-o", "--no-audio --loop --hwdec=auto --panscan=1.0", 
        "ALL", 
        path
    ]);

    println!("Executing: mpvpaper -f -o \"...\" ALL {}", path);
    let _ = cmd.spawn();
}

async fn handle_client(mut stream: UnixStream, state: std::sync::Arc<tokio::sync::Mutex<DaemonState>>, tx: tokio::sync::mpsc::Sender<IPCCommand>) {
    let mut buffer = [0u8; 4096];
    match stream.read(&mut buffer).await {
        Ok(n) if n > 0 => {
            let command: std::result::Result<IPCCommand, _> = serde_json::from_slice(&buffer[..n]);
            let response = match command {
                Ok(IPCCommand::SetWallpaper { path, wayland_display, hyprland_instance, transition, duration }) => {
                    let mut s = state.lock().await;
                    s.current_wallpaper = Some(path.clone());
                    let trans = transition.unwrap_or_else(|| s.default_transition.clone());
                    let dur = duration.unwrap_or(s.default_duration);
                    
                    println!("IPC Request: SetWallpaper {}", path);
                    let _ = tx.send(IPCCommand::SetWallpaper {
                        path: path.clone(),
                        wayland_display,
                        hyprland_instance,
                        transition: Some(trans),
                        duration: Some(dur),
                    }).await;
                    s.save();

                    if s.pywal_enabled {
                        let _ = tokio::process::Command::new("wal").args(&["-i", &path, "-n", "-e"]).spawn();
                    }
                    IPCResponse::Success(format!("Wallpaper set"))
                },
                Ok(IPCCommand::ListWallpapers) => {
                    let s = state.lock().await;
                    let mut walls = Vec::new();
                    if let Ok(entries) = std::fs::read_dir(&s.wallpapers_dir) {
                        for entry in entries.flatten() {
                            let path = entry.path();
                            if path.is_file() { walls.push(path.to_string_lossy().to_string()); }
                        }
                    }
                    IPCResponse::WallpaperList(walls)
                },
                Ok(IPCCommand::GetStatus) => {
                    let s = state.lock().await;
                    IPCResponse::Status {
                        wallpaper: s.current_wallpaper.clone(),
                        pywal: s.pywal_enabled,
                        wallpapers_dir: s.wallpapers_dir.to_string_lossy().to_string(),
                        default_transition: s.default_transition.clone(),
                        default_duration: s.default_duration,
                    }
                },
                Ok(IPCCommand::SetPywal(enabled)) => {
                    let mut s = state.lock().await;
                    s.pywal_enabled = enabled;
                    s.save();
                    IPCResponse::Success(format!("Pywal set to {}", enabled))
                },
                Ok(IPCCommand::SetDefaultTransition { transition, duration }) => {
                    let mut s = state.lock().await;
                    s.default_transition = transition;
                    s.default_duration = duration;
                    s.save();
                    IPCResponse::Success(format!("Default transition updated"))
                },
                Ok(IPCCommand::SetWallpaperDir(dir)) => {
                    let mut s = state.lock().await;
                    let path = PathBuf::from(dir);
                    if path.exists() && path.is_dir() {
                        s.wallpapers_dir = path;
                        s.save();
                        IPCResponse::Success(format!("Wallpaper directory updated to: {:?}", s.wallpapers_dir))
                    } else {
                        IPCResponse::Error("Directory does not exist".to_string())
                    }
                },
                Ok(IPCCommand::GetWallpapersDir) => {
                    let s = state.lock().await;
                    IPCResponse::WallpaperDir(s.wallpapers_dir.to_string_lossy().to_string())
                },
                Ok(_) => IPCResponse::Success("Acknowledged".to_string()),
                Err(e) => IPCResponse::Error(format!("Invalid command: {}", e)),
            };
            let response_bytes = serde_json::to_vec(&response).unwrap();
            let _ = stream.write_all(&response_bytes).await;
            let _ = stream.shutdown().await;
        },
        _ => {}
    }
}
