slint::include_modules!();
use tokio::net::UnixStream;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[path = "../common.rs"]
mod common;
use common::{IPCCommand, IPCResponse};

mod video;

thread_local! {
    static ALL_WALLPAPERS: std::cell::RefCell<Vec<WallpaperData>> = std::cell::RefCell::new(Vec::new());
}

#[tokio::main]
async fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let window = AppWindow::new()?;
    let handle = window.as_weak();
    let socket_path_string = common::get_socket_path();
    let socket_path: &'static str = Box::leak(socket_path_string.into_boxed_str());

    // Connect search
    let handle_search = handle.clone();
    window.on_search_wallpapers(move |query: slint::SharedString| {
        let q = query.to_string().to_lowercase();
        ALL_WALLPAPERS.with(|w| {
            let filtered: Vec<WallpaperData> = if q.is_empty() {
                w.borrow().clone()
            } else {
                w.borrow().iter().filter(|wall| wall.path.to_string().to_lowercase().contains(&q)).cloned().collect()
            };
            if let Some(h) = handle_search.upgrade() {
                h.set_wallpapers(std::rc::Rc::new(slint::VecModel::from(filtered)).into());
            }
        });
    });

    // Connect Video Previews
    let cancel_flag = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let active_task = std::sync::Arc::new(tokio::sync::Mutex::new(None::<tokio::task::JoinHandle<()>>));
    
    let handle_opened = handle.clone();
    let cancel_opened = cancel_flag.clone();
    let task_opened = active_task.clone();
    window.on_preview_opened(move |path: slint::SharedString| {
        let path_str = path.to_string();
        let ext = std::path::Path::new(&path_str).extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
        
        let cancel = cancel_opened.clone();
        let handle_weak = handle_opened.clone();
        let tasks = task_opened.clone();
        
        tokio::spawn(async move {
            cancel.store(true, std::sync::atomic::Ordering::Relaxed);
            let mut current = tasks.lock().await;
            if let Some(t) = current.take() { t.abort(); }
            
            if ext == "mp4" || ext == "mkv" {
                cancel.store(false, std::sync::atomic::Ordering::Relaxed);
                *current = Some(tokio::task::spawn_blocking(move || {
                    let _ = video::spawn_video_player(handle_weak, path_str, cancel);
                }));
            }
        });
    });
    
    let cancel_closed = cancel_flag.clone();
    let task_closed = active_task.clone();
    window.on_preview_closed(move || {
        let cancel = cancel_closed.clone();
        let tasks = task_closed.clone();
        tokio::spawn(async move {
            cancel.store(true, std::sync::atomic::Ordering::Relaxed);
            let mut current = tasks.lock().await;
            if let Some(t) = current.take() { t.abort(); }
        });
    });

    // Load initial wallpapers (non-blocking)
    let handle_init = handle.clone();
    tokio::spawn(async move {
        refresh_ui_wallpapers(handle_init, socket_path).await;
    });

    // Set callbacks
    window.on_set_wallpaper(move |path: slint::SharedString, trans: slint::SharedString, dur: i32| {
        let path_str = path.to_string();
        let transition_type = trans.to_string();
        let duration_ms = dur as u32;
        let wayland_display = std::env::var("WAYLAND_DISPLAY").ok();
        let hyprland_instance = std::env::var("HYPRLAND_INSTANCE_SIGNATURE").ok();
        tokio::spawn(async move {
            let cmd = IPCCommand::SetWallpaper {
                path: path_str,
                wayland_display,
                hyprland_instance,
                transition: Some(transition_type),
                duration: Some(duration_ms),
            };
            let _ = send_command(socket_path, cmd).await;
        });
    });

    let handle_refresh = handle.clone();
    window.on_refresh_wallpapers(move || {
        let h_copy = handle_refresh.clone();
        tokio::spawn(async move {
            refresh_ui_wallpapers(h_copy, socket_path).await;
        });
    });

    window.on_toggle_pywal(move |enabled: bool| {
        tokio::spawn(async move {
            let _ = send_command(socket_path, IPCCommand::SetPywal(enabled)).await;
        });
    });

    window.on_toggle_preview(move |enabled: bool| {
        tokio::spawn(async move {
            let _ = send_command(socket_path, IPCCommand::TogglePreview(enabled)).await;
        });
    });

    window.on_set_fill_mode(move |mode: slint::SharedString| {
        let fill = match mode.as_str() {
            "Crop" => common::WallpaperFill::Crop,
            "Fit" => common::WallpaperFill::Fit,
            "Stretch" => common::WallpaperFill::Stretch,
            "Center" => common::WallpaperFill::Center,
            "Tile" => common::WallpaperFill::Tile,
            _ => common::WallpaperFill::Crop,
        };
        tokio::spawn(async move {
            let _ = send_command(socket_path, IPCCommand::SetFill(fill)).await;
        });
    });

    window.on_update_default_transition(move |trans, dur| {
        let transition = trans.to_string();
        let duration = dur as u32;
        tokio::spawn(async move {
            let _ = send_command(socket_path, IPCCommand::SetDefaultTransition { transition, duration }).await;
        });
    });

    window.on_set_wallpaper_dir(move |dir| {
        let directory = dir.to_string();
        tokio::spawn(async move {
            let _ = send_command(socket_path, IPCCommand::SetWallpaperDir(directory)).await;
        });
    });

    // Initial status fetch
    let handle_status = handle.clone();
    tokio::spawn(async move {
        if let Ok(mut stream) = UnixStream::connect(socket_path).await {
            let _ = stream.write_all(&serde_json::to_vec(&IPCCommand::GetStatus).unwrap()).await;
            let _ = stream.shutdown().await;
            let mut buffer = Vec::new();
            if let Ok(_) = stream.read_to_end(&mut buffer).await {
                if let Ok(IPCResponse::Status { wallpaper: _, pywal, preview_enabled, wallpapers_dir, default_transition, default_duration }) = serde_json::from_slice::<IPCResponse>(&buffer) {
                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(h) = handle_status.upgrade() {
                            h.set_pywal_enabled(pywal);
                            h.set_enable_preview(preview_enabled);
                            h.set_wallpapers_dir(wallpapers_dir.into());
                            h.set_transition_type(default_transition.into());
                            h.set_transition_duration(default_duration as i32);
                        }
                    });
                }
            }
        }
    });

    // Color update loop (sync with Pywal)
    let handle_colors = handle.clone();
    tokio::spawn(async move {
        let wal_colors_path = dirs::home_dir().unwrap().join(".cache/wal/colors.json");
        let walllust_colors_path = dirs::cache_dir().unwrap().join("walllust/colors.json");
        loop {
            tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
            let path = if wal_colors_path.exists() { &wal_colors_path } else { &walllust_colors_path };
            if let Ok(content) = std::fs::read_to_string(path) {
                if let Ok(colors_obj) = serde_json::from_str::<serde_json::Value>(&content) {
                    let mut colors = Vec::new();
                    if let Some(c_obj) = colors_obj.get("colors") {
                        for i in 0..16 {
                            if let Some(c) = c_obj.get(format!("color{}", i)) {
                                if let Some(s) = c.as_str() { colors.push(s.to_string()); }
                            }
                        }
                    } else if let Ok(c_list) = serde_json::from_value::<Vec<String>>(colors_obj) {
                        colors = c_list;
                    }
                    if colors.len() >= 8 {
                        let h_copy = handle_colors.clone();
                        let _ = slint::invoke_from_event_loop(move || {
                            if let Some(h) = h_copy.upgrade() {
                                h.set_background_color(parse_color(&colors[0]));
                                h.set_accent_color(parse_color(&colors[4]));
                                h.set_text_color(parse_color(&colors[7]));
                                h.set_secondary_color(parse_color(&colors[1]));
                            }
                        });
                    }
                }
            }
        }
    });

    window.run()?;
    Ok(())
}

async fn fetch_wallpapers(socket_path: &str) -> std::result::Result<Vec<String>, Box<dyn std::error::Error>> {
    let mut stream = UnixStream::connect(socket_path).await?;
    let _ = stream.write_all(&serde_json::to_vec(&IPCCommand::ListWallpapers)?).await?;
    let _ = stream.shutdown().await?;
    let mut buffer = Vec::new();
    let _ = stream.read_to_end(&mut buffer).await?;
    if let IPCResponse::WallpaperList(walls) = serde_json::from_slice(&buffer)? { Ok(walls) } else { Ok(vec![]) }
}

async fn send_command(socket_path: &str, cmd: IPCCommand) -> std::result::Result<(), Box<dyn std::error::Error>> {
    let mut stream = UnixStream::connect(socket_path).await?;
    let _ = stream.write_all(&serde_json::to_vec(&cmd)?).await?;
    Ok(())
}

fn parse_color(hex: &str) -> slint::Color {
    if hex.starts_with('#') && hex.len() == 7 {
        let r = u8::from_str_radix(&hex[1..3], 16).unwrap_or(0);
        let g = u8::from_str_radix(&hex[3..5], 16).unwrap_or(0);
        let b = u8::from_str_radix(&hex[5..7], 16).unwrap_or(0);
        slint::Color::from_rgb_u8(r, g, b)
    } else { slint::Color::from_rgb_u8(0, 0, 0) }
}

async fn refresh_ui_wallpapers(handle: slint::Weak<AppWindow>, socket_path: &'static str) {
    let wallpapers = fetch_wallpapers(socket_path).await.unwrap_or_default();
    let cache_dir = dirs::cache_dir().unwrap().join("walllust/thumbnails");
    let _ = std::fs::create_dir_all(&cache_dir);
    let mut tasks = Vec::new();
    for path in wallpapers {
        let path_buf = std::path::PathBuf::from(&path);
        let ext = path_buf.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
        let thumb_path = cache_dir.join(format!("{:x}.jpg", fxhash::hash64(&path)));
        let thumb_path_str = thumb_path.to_str().unwrap().to_string();
        if !thumb_path.exists() {
            if ext == "mp4" || ext == "mkv" {
                let _ = std::process::Command::new("ffmpeg").args(&["-i", &path, "-ss", "00:00:01", "-vframes", "1", "-s", "320x180", "-f", "image2", &thumb_path_str]).output();
            } else if ["jpg", "jpeg", "png", "webp"].contains(&ext.as_str()) {
                if let Ok(img) = image::open(&path) { let _ = img.thumbnail(320, 180).save(&thumb_path_str); }
            }
        }
        tasks.push((path, thumb_path_str));
    }
    let _ = slint::invoke_from_event_loop(move || {
        if let Some(h) = handle.upgrade() {
            let wall_models: Vec<WallpaperData> = tasks.into_iter().map(|(orig_path, thumb_path)| {
                let thumbnail = slint::Image::load_from_path(std::path::Path::new(&thumb_path)).unwrap_or_default();
                WallpaperData { path: orig_path.into(), thumbnail }
            }).collect();
            
            ALL_WALLPAPERS.with(|w| { *w.borrow_mut() = wall_models.clone(); });
            let query = h.get_search_query().to_string().to_lowercase();
            let filtered: Vec<WallpaperData> = if query.is_empty() { 
                wall_models 
            } else { 
                wall_models.into_iter().filter(|w| w.path.to_string().to_lowercase().contains(&query)).collect() 
            };
            
            h.set_wallpapers(std::rc::Rc::new(slint::VecModel::from(filtered)).into());
        }
    });
}
