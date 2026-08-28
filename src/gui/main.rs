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
    // Leaked once at startup so the many 'static callback closures can share it.
    let socket_path: &'static str = Box::leak(common::get_socket_path().into_boxed_str());

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
            if let Err(e) = send_command(socket_path, cmd).await { eprintln!("Failed to send SetWallpaper command: {}", e); }
        });
    });

    let handle_refresh = handle.clone();
    window.on_refresh_wallpapers(move || {
        let h_copy = handle_refresh.clone();
        tokio::spawn(async move {
            refresh_ui_wallpapers(h_copy, socket_path).await;
        });
    });

    window.on_open_web_editor(move || {
        let _ = std::process::Command::new("xdg-open").arg("http://127.0.0.1:34567").spawn();
    });

    {
        let h = handle.clone();
        tokio::spawn(async move {
            use warp::Filter;
            use warp::Reply;
            
            let html_route = warp::path::end().map(|| {
                warp::reply::html(include_str!("../../assets/editor.html")).into_response()
            });

            #[derive(serde::Deserialize, serde::Serialize, Clone)]
            struct ExportObject {
                #[serde(rename = "type")]
                obj_type: String,
                left: f64,
                top: f64,
                width: f64,
                height: f64,
                angle: f64,
                fill: Option<String>,
                opacity: f64,
                #[serde(rename = "audioReactive")]
                audio_reactive: bool,
            }

            #[derive(serde::Deserialize, serde::Serialize, Clone)]
            struct ExportPayload {
                name: String,
                bg_color: String,
                objects: Vec<ExportObject>,
                has_media: bool,
                media_filename: Option<String>,
            }

            let h_clone = h.clone();
            let export_scene = warp::path!("api" / "export-scene")
                .and(warp::post())
                .and(warp::body::json())
                .map(move |payload: ExportPayload| {
                    // payload.name becomes a directory component, so reduce it to a
                    // bare file name — an attacker POSTing "../../foo" must not be
                    // able to create/overwrite directories outside live_scenes.
                    let safe_name = std::path::Path::new(&payload.name)
                        .file_name()
                        .map(|s| s.to_string_lossy().into_owned())
                        .filter(|s| !s.is_empty())
                        .unwrap_or_else(|| "pro_editor_scene".to_string());

                    // Create the scene directory
                    let mut scene_dir = dirs::config_dir().unwrap_or_else(|| std::path::PathBuf::from("."));
                    scene_dir.push("walllust");
                    scene_dir.push("live_scenes");
                    scene_dir.push(&safe_name);
                    let _ = std::fs::create_dir_all(&scene_dir);

                    // Save scene.json
                    let scene_json = serde_json::to_string_pretty(&payload).unwrap();
                    let _ = std::fs::write(scene_dir.join("scene.json"), scene_json);

                    // Tell GUI to refresh
                    let hc = h_clone.clone();
                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(app) = hc.upgrade() {
                            app.invoke_build_custom_wallpaper(
                                slint::SharedString::from(safe_name.clone()),
                                slint::SharedString::from(payload.bg_color.clone()),
                                slint::SharedString::from("FabricScene"),
                                slint::SharedString::from("#ffffff"),
                                false,
                            );
                        }
                    });
                    
                    warp::reply().into_response()
                });

            let export_media = warp::path!("api" / "export-media")
                .and(warp::post())
                .and(warp::header::<String>("X-Filename"))
                .and(warp::header::optional::<String>("X-Scene-Name"))
                .and(warp::body::bytes())
                .map(|filename: String, scene_name: Option<String>, body: warp::hyper::body::Bytes| {
                    // Both header values are attacker-controllable (any local
                    // process can POST here). Reduce each to a bare file name so a
                    // crafted value like "../../.bashrc" cannot escape the scene
                    // directory and write arbitrary paths.
                    let safe_scene = scene_name
                        .as_deref()
                        .and_then(|s| std::path::Path::new(s).file_name())
                        .map(|s| s.to_string_lossy().into_owned())
                        .filter(|s| !s.is_empty())
                        .unwrap_or_else(|| "pro_editor_scene".to_string());
                    let safe_file = std::path::Path::new(&filename)
                        .file_name()
                        .map(|s| s.to_string_lossy().into_owned());
                    let Some(safe_file) = safe_file.filter(|s| !s.is_empty()) else {
                        return warp::reply::with_status(
                            "invalid filename",
                            warp::http::StatusCode::BAD_REQUEST,
                        )
                        .into_response();
                    };

                    let mut scene_dir = dirs::config_dir().unwrap_or_else(|| std::path::PathBuf::from("."));
                    scene_dir.push("walllust");
                    scene_dir.push("live_scenes");
                    scene_dir.push(&safe_scene);

                    let _ = std::fs::create_dir_all(&scene_dir);
                    let _ = std::fs::write(scene_dir.join(safe_file), body);
                    warp::reply().into_response()
                });

            let routes = html_route.or(export_scene).or(export_media);
            warp::serve(routes).run(([127, 0, 0, 1], 34567)).await;
        });
    }

    window.on_toggle_pywal(move |enabled: bool| {
        tokio::spawn(async move {
            if let Err(e) = send_command(socket_path, IPCCommand::SetPywal(enabled)).await { eprintln!("Failed to send command: {}", e); }
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
            if let Err(e) = send_command(socket_path, IPCCommand::SetFill(fill)).await { eprintln!("Failed to send command: {}", e); }
        });
    });

    window.on_update_default_transition(move |trans, dur| {
        let transition = trans.to_string();
        let duration = dur as u32;
        tokio::spawn(async move {
            if let Err(e) = send_command(socket_path, IPCCommand::SetDefaultTransition { transition, duration }).await { eprintln!("Failed to send command: {}", e); }
        });
    });

    window.on_set_wallpaper_dir(move |dir| {
        let directory = dir.to_string();
        tokio::spawn(async move {
            if let Err(e) = send_command(socket_path, IPCCommand::SetWallpaperDir(directory)).await { eprintln!("Failed to send command: {}", e); }
        });
    });

    {
        let h = handle.clone();
        window.on_history_toggled(move |visible: bool| {
            let h2 = h.clone();
            let sp = socket_path;
            tokio::spawn(async move {
                if visible {
                    if let Ok(mut stream) = UnixStream::connect(sp).await {
                        let _ = stream.write_all(&serde_json::to_vec(&IPCCommand::GetHistory).unwrap()).await;
                        let _ = stream.shutdown().await;
                        let mut buffer = Vec::new();
                        if let Ok(_) = stream.read_to_end(&mut buffer).await {
                            if let Ok(IPCResponse::History(entries)) = serde_json::from_slice(&buffer) {
                                let entries_vec: Vec<slint::SharedString> = entries.iter()
                                    .map(|e| e.path.clone().into())
                                    .collect();
                                let h3 = h2.clone();
                                let _ = slint::invoke_from_event_loop(move || {
                                    let handle = h3.upgrade().unwrap();
                                    handle.set_history_entries(std::rc::Rc::new(slint::VecModel::from(entries_vec)).into());
                                    handle.set_history_visible(true);
                                });
                            }
                        }
                    }
                }
            });
        });
    }

    {
        let h = handle.clone();
        window.on_revert_wallpaper(move || {
            let path_str = socket_path.to_string();
            let h2 = h.clone();
            tokio::spawn(async move {
                if let Err(e) = send_command(&path_str, IPCCommand::RevertHistory).await { eprintln!("Failed to send command: {}", e); }
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(handle) = h2.upgrade() {
                        handle.set_history_visible(false);
                    }
                });
            });
        });
    }

    window.on_schedule_set(move |time: slint::SharedString, path: slint::SharedString| {
        let t = time.to_string();
        let p = path.to_string();
        tokio::spawn(async move {
            if let Err(e) = send_command(socket_path, IPCCommand::ScheduleSet { time: t, path: p }).await { eprintln!("Failed to send command: {}", e); }
        });
    });

    {
        let h = handle.clone();
        window.on_schedule_toggle(move |enabled: bool| {
            let h2 = h.clone();
            tokio::spawn(async move {
                if let Err(e) = send_command(socket_path, IPCCommand::ToggleSchedule).await { eprintln!("Failed to send command: {}", e); }
                let h3 = h2.clone();
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(handle) = h3.upgrade() {
                        handle.set_schedule_enabled(enabled);
                    }
                });
            });
        });
    }


    {
        let h = handle.clone();
        window.on_build_custom_wallpaper(move |name: slint::SharedString, bg_color: slint::SharedString, elem_type: slint::SharedString, elem_color: slint::SharedString, audio_reactive: bool| {
            let mut name_str = name.to_string();
            if name_str.trim().is_empty() {
                name_str = "my_scene".to_string();
            }
            let safe_name = name_str.replace(|c: char| !c.is_alphanumeric() && c != '-', "_");
            let filename = format!("{}.slint", safe_name);
            
            if let Some(handle_upgrade) = h.upgrade() {
                let mut dir = handle_upgrade.get_wallpapers_dir().to_string();
                if dir.is_empty() {
                    dir = dirs::home_dir().unwrap().join("Pictures/wallpapers").to_string_lossy().to_string();
                }
                
                let path = std::path::Path::new(&dir).join(&filename);
                
                let bg_col = if bg_color.trim().is_empty() { "#11111b" } else { bg_color.as_str() };
                let el_col = if elem_color.trim().is_empty() { "#00ffff" } else { elem_color.as_str() };
                
                let mut audio_props = String::new();
                let mut size_expr = "100px".to_string();
                
                if audio_reactive {
                    audio_props = "in-out property <float> audio_0;\n    in-out property <float> audio_1;".to_string();
                    size_expr = "(100px + audio_0 * 150px)".to_string();
                }
                
                let element_code = match elem_type.as_str() {
                    "Clock" => format!(r#"
    Text {{
        text: Math.floor(time_s / 3600) + ":" + Math.floor(mod(time_s / 60, 60)) + ":" + Math.floor(mod(time_s, 60));
        color: {el_col};
        font-size: {size_expr};
        font-weight: 800;
        horizontal-alignment: center;
        vertical-alignment: center;
        x: cursor_x * 0.05 * 1px;
        y: cursor_y * 0.05 * 1px;
    }}
"#),
                    "Bouncing Box" => format!(r#"
    Rectangle {{
        width: {size_expr};
        height: {size_expr};
        background: {el_col};
        border-radius: 20px;
        x: parent.width / 2 - self.width / 2 + Math.sin(time_s * 2.0) * (parent.width / 4);
        y: parent.height / 2 - self.height / 2 + Math.cos(time_s * 3.0) * (parent.height / 4);
    }}
"#),
                    _ => format!(r#"
    Rectangle {{
        width: {size_expr};
        height: {size_expr};
        background: {el_col};
        border-radius: {size_expr} / 2;
        x: cursor_x * 1px - self.width / 2;
        y: cursor_y * 1px - self.height / 2;
        animate x, y {{ duration: 150ms; easing: ease-out; }}
    }}
"#), // Defaults to Cursor Follower
                };

                let template = format!(r#"export component Main inherits Window {{
    in-out property <float> time_ms;
    in-out property <float> time_s;
    in-out property <float> cursor_x;
    in-out property <float> cursor_y;
    {}
    
    background: {};

{}
}}
"#, audio_props, bg_col, element_code);

                let _res = std::fs::write(&path, template);
                
                // Immediately refresh wallpaper list
                handle_upgrade.invoke_refresh_wallpapers();
                
                // Also automatically "install" (set) the newly created wallpaper
                let path_str = path.to_string_lossy().to_string();
                let trans = handle_upgrade.get_transition_type();
                let dur = handle_upgrade.get_transition_duration();
                handle_upgrade.invoke_set_wallpaper(slint::SharedString::from(path_str), trans, dur);
            }
        });
    }


    // Initial status fetch
    let handle_status = handle.clone();
    tokio::spawn(async move {
        if let Ok(mut stream) = UnixStream::connect(socket_path).await {
            let _ = stream.write_all(&serde_json::to_vec(&IPCCommand::GetStatus).unwrap()).await;
            let _ = stream.shutdown().await;
            let mut buffer = Vec::new();
            if let Ok(_) = stream.read_to_end(&mut buffer).await {
                if let Ok(IPCResponse::Status { wallpaper: _, pywal, preview_enabled, wallpapers_dir, default_transition, default_duration, .. }) = serde_json::from_slice::<IPCResponse>(&buffer) {
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

    // Start the asynchronous file system watcher task for color updates
    let handle_colors_clone = handle.clone();
    tokio::spawn(async move {
        // NOTE: In a real implementation, replace this placeholder with a robust
        // filesystem watcher (e.g., using the 'notify' crate with tokio)
        // that monitors the directories defined below for changes to colors.json.
        // When a change is detected, call update_colors.
        // For demonstration, we simulate the initial check and rely on external setup 
        // for event-driven updates.
        
        // Initial check of colors
        update_colors(handle_colors_clone.clone()).await;
        
        // *** Watcher Implementation Placeholder ***
        // Implementation detail: Setup Watcher over paths:
        // 1. dirs::home_dir().unwrap().join(".cache/wal/")
        // 2. dirs::cache_dir().unwrap().join("walllust/")
        // On event (Write/Rename/Create) to colors.json:
        //     update_colors(handle_colors_clone.clone()).await;
    });

    // Slint's event loop is blocking; it returns when the window closes.
    window.run()?;
    Ok(())
}

async fn send_command(socket_path: &str, cmd: IPCCommand) -> anyhow::Result<IPCResponse> {
    let mut stream = UnixStream::connect(socket_path).await?;
    stream.write_all(&serde_json::to_vec(&cmd)?).await?;
    stream.shutdown().await?;
    let mut buffer = Vec::new();
    stream.read_to_end(&mut buffer).await?;
    Ok(serde_json::from_slice(&buffer)?)
}

async fn fetch_wallpapers(socket_path: &str) -> anyhow::Result<Vec<String>> {
    if let IPCResponse::WallpaperList(walls) = send_command(socket_path, IPCCommand::ListWallpapers).await? {
        Ok(walls)
    } else {
        Ok(vec![])
    }
}

async fn update_colors(handle_colors: slint::Weak<AppWindow>) {
    let wal_colors_path = dirs::home_dir().unwrap().join(".cache/wal/colors.json");
    let walllust_colors_path = dirs::cache_dir().unwrap().join("walllust/colors.json");
    let path = if wal_colors_path.exists() { wal_colors_path } else { walllust_colors_path };

    if path.exists() {
        let content_result = tokio::task::spawn_blocking(move || std::fs::read_to_string(&path)).await.unwrap();
        if let Ok(content) = content_result {
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
}

fn parse_color(hex: &str) -> slint::Color {
    if hex.starts_with('#') && hex.len() == 7 {
        let r = u8::from_str_radix(&hex[1..3], 16).unwrap_or(0);
        let g = u8::from_str_radix(&hex[3..5], 16).unwrap_or(0);
        let b = u8::from_str_radix(&hex[5..7], 16).unwrap_or(0);
        slint::Color::from_rgb_u8(r, g, b)
    } else { slint::Color::from_rgb_u8(0, 0, 0) }
}

async fn refresh_ui_wallpapers(handle: slint::Weak<AppWindow>, socket_path: &str) {
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
