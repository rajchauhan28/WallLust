use clap::Parser;
use layer_shika::prelude::*;
use layer_shika::slint::ComponentHandle;
use std::path::{Path, PathBuf};
use std::process::Child;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};

mod audio;

#[path = "../common.rs"]
mod common;
use common::{HistoryEntry, IPCCommand, IPCResponse};

const VIDEO_EXTENSIONS: &[&str] = &[
    "mp4", "mkv", "webm", "mov", "avi", "flv", "wmv", "mpg", "mpeg", "gif",
];
// Explicit whitelist. Previously "image" meant "anything that is not a video
// and not a scene", so a .zip or .rdp sitting in the wallpaper folder was
// handed to the image renderer and listed as a wallpaper.
const IMAGE_EXTENSIONS: &[&str] = &[
    "jpg", "jpeg", "png", "webp", "bmp", "tif", "tiff", "avif", "jxl", "qoi", "pnm", "tga",
];
const SCENE_EXTENSION: &str = "slint";
const WALLPAPER_COMPONENT: &str = "Wallpaper";
// User scene files must export a component with this name (layer-shika's default).
const SCENE_COMPONENT: &str = "Main";

fn get_home_path() -> Option<PathBuf> {
    dirs::home_dir()
}

/// Minute-of-day (0..1439) in the machine's local timezone. Schedule entries are
/// `HH:MM` wall-clock times, so they must be compared against local time, not UTC.
fn local_minute_of_day() -> u64 {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0) as libc::time_t;
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    if unsafe { libc::localtime_r(&now, &mut tm) }.is_null() {
        // Fall back to UTC if the timezone lookup fails.
        return ((now as u64) % 86400) / 60;
    }
    (tm.tm_hour as u64) * 60 + (tm.tm_min as u64)
}

/// Arranges for `cmd`'s child to receive SIGKILL if this daemon process dies,
/// so scene/video renderers can never outlive the daemon that owns them.
fn die_with_parent(cmd: &mut std::process::Command) {
    use std::os::unix::process::CommandExt;
    unsafe {
        cmd.pre_exec(|| {
            libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL);
            Ok(())
        });
    }
}

fn get_runtime_dir() -> String {
    std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| {
        if let Some(home) = get_home_path() {
            format!("{}/.local/share", home.display())
        } else {
            "/tmp".to_string()
        }
    })
}

fn sync_lock_screen_wallpaper(wallpaper_path: &str) {
    if let Some(home) = get_home_path() {
        let lightdm_path = home.join(".config/lightdm/.wallpaper");
        if lightdm_path.parent().map(|p| p.exists()).unwrap_or(false) {
            let _ = std::fs::write(&lightdm_path, wallpaper_path);
        }
    }
}

#[derive(Parser)]
#[command(name = "walllust-daemon")]
#[command(about = "Wallpaper daemon for walllust", long_about = None)]
struct Args {
    /// Wallpapers directory
    #[arg(short, long)]
    wallpapers_dir: Option<PathBuf>,

    /// Disable Pywal integration
    #[arg(long)]
    no_pywal: bool,

    /// Default transition type (fade, slide-left, slide-right, slide-up, slide-down, zoom-in, zoom-out)
    #[arg(short, long, default_value = "fade")]
    transition: String,

    /// Default transition duration in milliseconds
    #[arg(short, long, default_value_t = 1000)]
    duration: u32,

    /// Internal: render a .slint scene wallpaper (spawned by the daemon itself)
    #[arg(long, hide = true)]
    render_scene: Option<PathBuf>,
}

// `#[serde(default)]` on every field so a config written by an older or newer
// walllust (with fields added/removed) still loads, preserving whatever settings
// it does contain instead of resetting to defaults.
#[derive(serde::Serialize, serde::Deserialize, Default)]
#[serde(default)]
struct DaemonState {
    current_wallpaper: Option<String>,
    pywal_enabled: bool,
    wallpapers_dir: PathBuf,
    default_transition: String,
    default_duration: u32,
    history: Vec<HistoryEntry>,
    schedule_enabled: bool,
    schedule: Vec<common::ScheduleEntry>,
}

impl DaemonState {
    fn save(&self) {
        if let Some(home) = get_home_path() {
            let config_dir = home.join(".config/walllust");
            if let Ok(json) = serde_json::to_string_pretty(self) {
                let _ = std::fs::create_dir_all(&config_dir);
                let _ = std::fs::write(config_dir.join("config.json"), json);
            }
        }
    }

    fn load(args: &Args) -> Self {
        let config_path = match get_home_path() {
            Some(h) => h.join(".config/walllust/config.json"),
            None => PathBuf::from("/tmp/walllust_config.json"),
        };
        if let Ok(content) = std::fs::read_to_string(&config_path) {
            if let Ok(state) = serde_json::from_str::<DaemonState>(&content) {
                return state;
            }
        }

        let wallpapers_dir = args.wallpapers_dir.clone().unwrap_or_else(|| {
            get_home_path()
                .unwrap_or_else(|| PathBuf::from("/tmp"))
                .join("Pictures/wallpapers")
        });

        DaemonState {
            current_wallpaper: None,
            pywal_enabled: !args.no_pywal,
            wallpapers_dir,
            default_transition: args.transition.clone(),
            default_duration: args.duration,
            history: Vec::new(),
            schedule_enabled: false,
            schedule: Vec::new(),
        }
    }

    fn add_history_entry(&mut self, path: &str) {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        self.history.insert(
            0,
            HistoryEntry {
                path: path.to_string(),
                timestamp: now,
            },
        );
        if self.history.len() > 50 {
            self.history.truncate(50);
        }
    }
}

fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    if let Some(scene) = args.render_scene.clone() {
        return run_scene_renderer(&scene);
    }

    tokio::runtime::Runtime::new()?.block_on(run_daemon(args))
}

async fn run_daemon(args: Args) -> anyhow::Result<()> {
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
    {
        let s = state.lock().await;
        if !s.wallpapers_dir.exists() {
            let _ = std::fs::create_dir_all(&s.wallpapers_dir);
        }
    }

    // Clean up renderers orphaned by a previous daemon instance; from here on
    // they are tracked as child processes.
    let _ = std::process::Command::new("pkill").arg("mpvpaper").status();
    let _ = std::process::Command::new("pkill")
        .args(["-f", "walllust-daemon --render-scene"])
        .status();

    let (shell_tx, shell_rx) = tokio::sync::mpsc::channel::<IPCCommand>(32);

    let tx_for_shell = shell_tx.clone();
    std::thread::spawn(move || run_wallpaper_shell(tx_for_shell, shell_rx));

    let scheduler_state = state.clone();
    let scheduler_tx = shell_tx.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(60));
        loop {
            interval.tick().await;
            let (enabled, schedule, transition, duration) = {
                let s = scheduler_state.lock().await;
                (
                    s.schedule_enabled,
                    s.schedule.clone(),
                    s.default_transition.clone(),
                    s.default_duration,
                )
            };
            if enabled {
                check_schedule_times(&schedule, &scheduler_tx, &transition, duration).await;
            }
        }
    });

    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                println!("Received shutdown signal. Shutting down daemon...");
                break;
            }
            result = listener.accept() => {
                match result {
                    Ok((stream, _)) => {
                        let s = state.clone();
                        let sender = shell_tx.clone();
                        tokio::spawn(async move {
                            handle_client(stream, s, sender).await;
                        });
                    }
                    Err(e) => eprintln!("Error accepting connection: {}", e),
                }
            }
        }
    }

    let _ = std::fs::remove_file(&socket_path);
    println!("Daemon shut down gracefully.");
    Ok(())
}

async fn check_schedule_times(
    schedule: &[common::ScheduleEntry],
    tx: &tokio::sync::mpsc::Sender<IPCCommand>,
    transition: &str,
    duration: u32,
) {
    if schedule.is_empty() {
        return;
    }

    let now_min = local_minute_of_day();

    for entry in schedule {
        let parts: Vec<&str> = entry.time.split(':').collect();
        if parts.len() != 2 {
            continue;
        }
        if let (Ok(h), Ok(m)) = (parts[0].parse::<u64>(), parts[1].parse::<u64>()) {
            let entry_min = h * 60 + m;
            if (now_min as i64 - entry_min as i64).abs() <= 1 {
                println!("Schedule trigger: {} at {}", entry.path, entry.time);
                let cmd = IPCCommand::SetWallpaper {
                    path: entry.path.clone(),
                    wayland_display: None,
                    hyprland_instance: None,
                    transition: Some(transition.to_string()),
                    duration: Some(duration),
                };
                if let Err(e) = tx.send(cmd).await {
                    eprintln!("Failed to send schedule command: {}", e);
                }
            }
        }
    }
}

/// Runs the wallpaper layer-shell surface and dispatches commands onto its
/// event loop. Owns the external renderer children (mpvpaper for videos, a
/// `--render-scene` child of this binary for interactive scenes) so backend
/// handoffs kill exactly the process we started.
fn run_wallpaper_shell(
    internal_tx: tokio::sync::mpsc::Sender<IPCCommand>,
    mut rx: tokio::sync::mpsc::Receiver<IPCCommand>,
) {
    let ui_source = include_str!("../../ui/wallpaper.slint");

    println!("Initializing Layer Shell...");
    let mut shell = loop {
        match Shell::from_source(ui_source)
            .surface(WALLPAPER_COMPONENT)
            .layer(Layer::Bottom)
            .anchor(AnchorEdges::all())
            .output_policy(OutputPolicy::AllOutputs)
            .build()
        {
            Ok(s) => break s,
            Err(e) => {
                eprintln!("Failed to build layer shell: {e}. Retrying in 2 seconds...");
                std::thread::sleep(Duration::from_secs(2));
            }
        }
    };

    let mut mpv_child: Option<Child> = None;
    let mut scene_child: Option<Child> = None;

    let tx_for_events = internal_tx.clone();
    let (_token, event_tx) = shell
        .event_loop_handle()
        .add_channel::<IPCCommand, _>(move |cmd, app_state| match cmd {
            IPCCommand::SetWallpaper {
                path,
                wayland_display,
                hyprland_instance,
                transition,
                duration,
            } => {
                let path_buf = PathBuf::from(&path);
                let absolute_path = path_buf.canonicalize().unwrap_or(path_buf);
                let ext = absolute_path
                    .extension()
                    .and_then(|e| e.to_str())
                    .unwrap_or("")
                    .to_lowercase();
                let path_str = absolute_path.to_string_lossy().to_string();

                if VIDEO_EXTENSIONS.contains(&ext.as_str()) {
                    println!("Video wallpaper: {path_str}. Handing off to mpvpaper...");
                    kill_child(&mut scene_child);
                    kill_child(&mut mpv_child);
                    for surface in app_state.surfaces_by_name(WALLPAPER_COMPONENT) {
                        let component = surface.component_instance();
                        let _ = component
                            .set_property("surface_visible", slint_interpreter::Value::Bool(false));
                    }
                    mpv_child = spawn_mpvpaper(&path_str, wayland_display, hyprland_instance);
                    return;
                }

                if ext == SCENE_EXTENSION {
                    println!("Scene wallpaper: {path_str}. Launching scene renderer...");
                    kill_child(&mut scene_child);
                    kill_child(&mut mpv_child);
                    for surface in app_state.surfaces_by_name(WALLPAPER_COMPONENT) {
                        let component = surface.component_instance();
                        let _ = component
                            .set_property("surface_visible", slint_interpreter::Value::Bool(false));
                    }
                    scene_child =
                        spawn_scene_renderer(&absolute_path, wayland_display, hyprland_instance);
                    return;
                }

                if absolute_path.is_dir() && absolute_path.join("scene.json").exists() {
                    println!("Live web scene: {path_str}. Launching webkit renderer...");
                    kill_child(&mut scene_child);
                    kill_child(&mut mpv_child);
                    for surface in app_state.surfaces_by_name(WALLPAPER_COMPONENT) {
                        let component = surface.component_instance();
                        let _ = component
                            .set_property("surface_visible", slint_interpreter::Value::Bool(false));
                    }
                    
                    // Spawn walllust-renderer
                    if let Ok(exe) = std::env::current_exe() {
                        let dir = exe.parent().unwrap();
                        let renderer = dir.join("walllust-renderer");
                        
                        let mut cmd = std::process::Command::new(renderer);
                        cmd.arg(&absolute_path);
                        
                        if let Some(wd) = wayland_display.as_ref() { cmd.env("WAYLAND_DISPLAY", wd); }
                        if let Some(hi) = hyprland_instance.as_ref() { cmd.env("HYPRLAND_INSTANCE_SIGNATURE", hi); }
                        die_with_parent(&mut cmd);

                        if let Ok(child) = cmd.spawn() {
                            scene_child = Some(child);
                        } else {
                            eprintln!("Failed to spawn walllust-renderer!");
                        }
                    }
                    return;
                }

                println!("Image wallpaper: {path_str}");
                match slint::Image::load_from_path(&absolute_path) {
                    Ok(img) => {
                        kill_child(&mut scene_child);
                        kill_child(&mut mpv_child);

                        let trans_type = transition.unwrap_or_else(|| "fade".to_string());
                        let trans_dur = duration.unwrap_or(1000) as f64;

                        for surface in app_state.surfaces_by_name(WALLPAPER_COMPONENT) {
                            let component = surface.component_instance();
                            let is_1 = matches!(
                                component.get_property("active_is_1"),
                                Ok(slint_interpreter::Value::Bool(true))
                            );
                            let new_is_1 = !is_1;
                            let img_prop = if new_is_1 { "image1" } else { "image2" };

                            let _ = component.set_property(
                                "surface_visible",
                                slint_interpreter::Value::Bool(true),
                            );
                            let _ = component.set_property(
                                img_prop,
                                slint_interpreter::Value::Image(img.clone()),
                            );
                            let _ = component.set_property(
                                "transition_type",
                                slint_interpreter::Value::from(slint::SharedString::from(
                                    trans_type.as_str(),
                                )),
                            );
                            let _ = component.set_property(
                                "transition_duration_ms",
                                slint_interpreter::Value::Number(trans_dur),
                            );

                            // Let the new image land in the hidden slot before flipping,
                            // so the animation starts from the fully-hidden state.
                            let tx = tx_for_events.clone();
                            let trans_dur_ms = trans_dur as u64;
                            std::thread::spawn(move || {
                                std::thread::sleep(Duration::from_millis(2));
                                let _ = tx.blocking_send(IPCCommand::InternalFlip {
                                    new_is_1,
                                    trans_dur: trans_dur_ms,
                                });
                            });
                        }
                    }
                    Err(e) => eprintln!("Error loading image {path}: {e}"),
                }
            }
            IPCCommand::InternalFlip { new_is_1, trans_dur } => {
                for surface in app_state.surfaces_by_name(WALLPAPER_COMPONENT) {
                    let component = surface.component_instance();
                    let _ = component
                        .set_property("active_is_1", slint_interpreter::Value::Bool(new_is_1));
                }

                // Drive redraws for the duration of the transition (~60 fps).
                let tx = tx_for_events.clone();
                std::thread::spawn(move || {
                    let start = Instant::now();
                    loop {
                        std::thread::sleep(Duration::from_millis(16));
                        let elapsed = start.elapsed().as_millis() as u64;
                        let final_frame = elapsed >= trans_dur;
                        let sent = tx.blocking_send(IPCCommand::InternalRedraw {
                            dur: trans_dur,
                            elapsed,
                            final_frame,
                        });
                        if sent.is_err() || final_frame {
                            break;
                        }
                    }
                });
            }
            IPCCommand::InternalRedraw { final_frame, .. } => {
                if final_frame {
                    println!("Transition complete.");
                }
                for surface in app_state.surfaces_by_name(WALLPAPER_COMPONENT) {
                    let component = surface.component_instance();
                    component.window().request_redraw();
                }
            }
            _ => {}
        })
        .expect("Failed to register the IPC channel with the event loop");

    // Forward commands from the tokio side onto the event-loop thread.
    std::thread::spawn(move || {
        while let Some(cmd) = rx.blocking_recv() {
            if event_tx.send(cmd).is_err() {
                break;
            }
        }
    });

    println!("Starting Layer Shell event loop...");
    shell.run().expect("Layer shell event loop failed");
}

fn kill_child(slot: &mut Option<Child>) {
    if let Some(mut child) = slot.take() {
        let _ = child.kill();
        let _ = child.wait();
    }
}

fn spawn_mpvpaper(
    path: &str,
    wayland_display: Option<String>,
    hyprland_instance: Option<String>,
) -> Option<Child> {
    let mut cmd = std::process::Command::new("mpvpaper");

    if let Some(wd) = wayland_display {
        cmd.env("WAYLAND_DISPLAY", wd);
    }
    if let Some(hi) = hyprland_instance {
        cmd.env("HYPRLAND_INSTANCE_SIGNATURE", hi);
    }
    cmd.env("XDG_RUNTIME_DIR", get_runtime_dir());

    let current_path = std::env::var("PATH").unwrap_or_default();
    if let Some(home) = get_home_path() {
        let local_bin = home.join(".local/bin");
        cmd.env(
            "PATH",
            format!("{}:{}:/usr/local/bin:/usr/bin:/bin", current_path, local_bin.display()),
        );
    }

    // No -f: that is mpvpaper's fork flag. With it, the process we spawn
    // daemonizes and exits, so the tracked Child is a corpse — kill_child
    // kills nothing and every video set leaks a fullscreen renderer (observed:
    // five mpvpapers decoding at once). Foreground keeps it a real child, and
    // keeps die_with_parent effective (PDEATHSIG does not survive the fork).
    cmd.args([
        "-o",
        "--no-audio --loop --hwdec=auto --panscan=1.0",
        "ALL",
        path,
    ]);

    println!("Executing: mpvpaper -o \"...\" ALL {}", path);
    die_with_parent(&mut cmd);
    match cmd.spawn() {
        Ok(child) => Some(child),
        Err(e) => {
            eprintln!("Failed to launch mpvpaper: {e}");
            None
        }
    }
}

fn spawn_scene_renderer(
    path: &Path,
    wayland_display: Option<String>,
    hyprland_instance: Option<String>,
) -> Option<Child> {
    let exe = match std::env::current_exe() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("Cannot locate daemon executable for scene renderer: {e}");
            return None;
        }
    };

    let mut cmd = std::process::Command::new(exe);
    cmd.arg("--render-scene").arg(path);
    if let Some(wd) = wayland_display {
        cmd.env("WAYLAND_DISPLAY", wd);
    }
    if let Some(hi) = hyprland_instance {
        cmd.env("HYPRLAND_INSTANCE_SIGNATURE", hi);
    }
    die_with_parent(&mut cmd);

    match cmd.spawn() {
        Ok(child) => Some(child),
        Err(e) => {
            eprintln!("Failed to launch scene renderer: {e}");
            None
        }
    }
}

/// Renders a user-supplied .slint file as a live wallpaper in its own process,
/// so a broken scene can never take down the daemon.
///
/// Property contract (all optional — missing properties are ignored):
///   in-out property <float> time_ms;   // milliseconds since the scene started
///   in-out property <float> time_s;    // seconds since the scene started
///   in-out property <float> cursor_x;  // global cursor position (Hyprland only,
///   in-out property <float> cursor_y;  // updates even under windows)
///
/// Pointer events over uncovered desktop areas are delivered natively, so
/// TouchArea etc. work without any of these properties.
fn run_scene_renderer(scene_path: &Path) -> anyhow::Result<()> {
    use layer_shika::calloop::TimeoutAction;

    let scene_path = scene_path
        .canonicalize()
        .unwrap_or_else(|_| scene_path.to_path_buf());
    println!("Scene renderer starting for {}", scene_path.display());

    let mut shell = Shell::from_file(&scene_path)
        .surface(SCENE_COMPONENT)
        .layer(Layer::Bottom)
        .anchor(AnchorEdges::all())
        .output_policy(OutputPolicy::AllOutputs)
        .build()
        .map_err(|e| {
            anyhow::anyhow!(
                "failed to compile scene '{}' (it must export a component named '{}'): {e}",
                scene_path.display(),
                SCENE_COMPONENT
            )
        })?;

    let handle = shell.event_loop_handle();

    // Drive time into the scene and repaint at ~30 fps.
    let start = Instant::now();
    handle
        .add_timer(Duration::from_millis(33), move |_deadline, app_state| {
            let t = start.elapsed().as_millis() as f64;
            for surface in app_state.surfaces_by_name(SCENE_COMPONENT) {
                let component = surface.component_instance();
                let _ = component.set_property("time_ms", slint_interpreter::Value::Number(t));
                let _ =
                    component.set_property("time_s", slint_interpreter::Value::Number(t / 1000.0));
                component.window().request_redraw();
            }
            TimeoutAction::ToDuration(Duration::from_millis(33))
        })
        .map_err(|e| anyhow::anyhow!("failed to register scene timer: {e}"))?;

    // Feed the global cursor position so scenes can react even when windows
    // cover the desktop (the compositor only sends us pointer events over
    // bare desktop areas).
    if std::env::var("HYPRLAND_INSTANCE_SIGNATURE").is_ok() {
        let (_token, cursor_tx) = handle
            .add_channel::<(f64, f64), _>(|(x, y), app_state| {
                for surface in app_state.surfaces_by_name(SCENE_COMPONENT) {
                    let component = surface.component_instance();
                    let _ =
                        component.set_property("cursor_x", slint_interpreter::Value::Number(x));
                    let _ =
                        component.set_property("cursor_y", slint_interpreter::Value::Number(y));
                }
            })
            .map_err(|e| anyhow::anyhow!("failed to register cursor channel: {e}"))?;

        std::thread::spawn(move || {
            let mut last = (f64::NAN, f64::NAN);
            let mut confirmed = false;
            let mut fails = 0u32;
            loop {
                match query_hyprland_cursor() {
                    Some(pos) => {
                        if !confirmed {
                            println!("Cursor tracking active (first read: {}, {})", pos.0, pos.1);
                            confirmed = true;
                        }
                        if pos != last {
                            if cursor_tx.send(pos).is_err() {
                                return;
                            }
                            last = pos;
                        }
                    }
                    None => {
                        fails += 1;
                        if fails == 30 {
                            eprintln!(
                                "Cursor tracking: hyprctl cursorpos is not returning a position; \
                                 scenes using cursor_x/cursor_y will not react to the mouse."
                            );
                        }
                    }
                }
                std::thread::sleep(Duration::from_millis(33));
            }
        });
    }

    // Feed the live audio spectrum into audio_0..audio_7. Kept alive until the
    // renderer exits; PR_SET_PDEATHSIG in the capture child reaps parec.
    let (_audio_token, audio_tx) = handle
        .add_channel::<[f32; audio::BANDS], _>(|bands, app_state| {
            for surface in app_state.surfaces_by_name(SCENE_COMPONENT) {
                let component = surface.component_instance();
                for (i, v) in bands.iter().enumerate() {
                    let _ = component.set_property(
                        &format!("audio_{i}"),
                        slint_interpreter::Value::Number(*v as f64),
                    );
                }
                component.window().request_redraw();
            }
        })
        .map_err(|e| anyhow::anyhow!("failed to register audio channel: {e}"))?;
    let _audio_child = audio::start(move |bands| audio_tx.send(bands).map_err(|_| ()));

    shell
        .run()
        .map_err(|e| anyhow::anyhow!("scene event loop failed: {e}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    fn compile_slint_source(source: String) -> slint_interpreter::CompilationResult {
        let compiler = slint_interpreter::Compiler::default();
        tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(compiler.build_from_source(source, std::path::PathBuf::new()))
    }

    #[test]
    fn wallpaper_ui_compiles_and_exports_expected_component() {
        let result = compile_slint_source(include_str!("../../ui/wallpaper.slint").to_string());
        let diagnostics: Vec<_> = result.diagnostics().collect();
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
        assert!(
            result.component_names().any(|n| n == super::WALLPAPER_COMPONENT),
            "ui/wallpaper.slint must export a '{}' component",
            super::WALLPAPER_COMPONENT
        );
    }

    #[test]
    fn demo_scene_compiles_and_exports_main() {
        let source = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("scenes/demo.slint"),
        )
        .unwrap();
        let result = compile_slint_source(source);
        let diagnostics: Vec<_> = result.diagnostics().collect();
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
        assert!(
            result.component_names().any(|n| n == super::SCENE_COMPONENT),
            "scene files must export a '{}' component",
            super::SCENE_COMPONENT
        );
    }
}

fn query_hyprland_cursor() -> Option<(f64, f64)> {
    let out = std::process::Command::new("hyprctl")
        .arg("cursorpos")
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let mut parts = text.trim().split(',');
    let x = parts.next()?.trim().parse().ok()?;
    let y = parts.next()?.trim().parse().ok()?;
    Some((x, y))
}

async fn handle_client(
    mut stream: UnixStream,
    state: std::sync::Arc<tokio::sync::Mutex<DaemonState>>,
    tx: tokio::sync::mpsc::Sender<IPCCommand>,
) {
    let mut buffer = [0u8; 4096];
    match stream.read(&mut buffer).await {
        Ok(n) if n > 0 => {
            let command: std::result::Result<IPCCommand, _> =
                serde_json::from_slice(&buffer[..n]);
            let response = match command {
                Ok(IPCCommand::SetWallpaper {
                    path,
                    wayland_display,
                    hyprland_instance,
                    transition,
                    duration,
                }) => match std::fs::canonicalize(&path) {
                    Err(_) => IPCResponse::Error(format!("File not found: {}", path)),
                    Ok(canonical) => {
                        let path = canonical.to_string_lossy().to_string();
                        let ext = canonical
                            .extension()
                            .and_then(|e| e.to_str())
                            .unwrap_or("")
                            .to_lowercase();
                        let is_image = IMAGE_EXTENSIONS.contains(&ext.as_str());
                        let is_video = VIDEO_EXTENSIONS.contains(&ext.as_str());
                        let is_scene = ext == SCENE_EXTENSION;

                        if !is_image && !is_video && !is_scene {
                            // Reject rather than hand an archive or a stray
                            // download to the image renderer.
                            IPCResponse::Error(format!(
                                "Unsupported wallpaper type: .{} ({})",
                                ext, path
                            ))
                        } else {
                            let mut s = state.lock().await;
                            s.add_history_entry(&path);
                            s.current_wallpaper = Some(path.clone());
                            let trans = transition.unwrap_or_else(|| s.default_transition.clone());
                            let dur = duration.unwrap_or(s.default_duration);

                            println!("IPC Request: SetWallpaper {}", path);
                            let _ = tx
                                .send(IPCCommand::SetWallpaper {
                                    path: path.clone(),
                                    wayland_display,
                                    hyprland_instance,
                                    transition: Some(trans),
                                    duration: Some(dur),
                                })
                                .await;
                            s.save();

                            if s.pywal_enabled && is_image {
                                let _ = tokio::process::Command::new("wal")
                                    .args(["-i", &path, "-n", "-e"])
                                    .spawn();
                            }
                            if is_image {
                                sync_lock_screen_wallpaper(&path);
                            }
                            IPCResponse::Success("Wallpaper set".to_string())
                        }
                    }
                },
                Ok(IPCCommand::ListWallpapers) => {
                    let s = state.lock().await;
                    let mut walls = Vec::new();
                    if let Ok(entries) = std::fs::read_dir(&s.wallpapers_dir) {
                        for entry in entries.flatten() {
                            let path = entry.path();
                            if !path.is_file() {
                                continue;
                            }
                            // Only real media: the folder also collects
                            // archives and stray downloads.
                            let ext = path
                                .extension()
                                .and_then(|e| e.to_str())
                                .unwrap_or("")
                                .to_lowercase();
                            if IMAGE_EXTENSIONS.contains(&ext.as_str())
                                || VIDEO_EXTENSIONS.contains(&ext.as_str())
                                || ext == SCENE_EXTENSION
                            {
                                walls.push(path.to_string_lossy().to_string());
                            }
                        }
                    }
                    
                    // Scan live_scenes
                    let live_scenes = dirs::config_dir().unwrap_or_else(|| std::path::PathBuf::from(".")).join("walllust/live_scenes");
                    if let Ok(entries) = std::fs::read_dir(&live_scenes) {
                        for entry in entries.flatten() {
                            let path = entry.path();
                            if path.is_dir() && path.join("scene.json").exists() {
                                walls.push(path.to_string_lossy().to_string());
                            }
                        }
                    }
                    
                    IPCResponse::WallpaperList(walls)
                }
                Ok(IPCCommand::GetStatus) => {
                    let s = state.lock().await;
                    IPCResponse::Status {
                        wallpaper: s.current_wallpaper.clone(),
                        pywal: s.pywal_enabled,
                        wallpapers_dir: s.wallpapers_dir.to_string_lossy().to_string(),
                        default_transition: s.default_transition.clone(),
                        default_duration: s.default_duration,
                        history: s.history.clone(),
                        schedule_enabled: s.schedule_enabled,
                    }
                }
                Ok(IPCCommand::GetHistory) => {
                    let s = state.lock().await;
                    IPCResponse::History(s.history.clone())
                }
                Ok(IPCCommand::RevertHistory) => {
                    let mut s = state.lock().await;
                    if let Some(entry) = s.history.get(1).map(|e| e.path.clone()) {
                        s.current_wallpaper = Some(entry.clone());
                        s.save();
                        let trans = s.default_transition.clone();
                        let dur = s.default_duration;
                        drop(s);
                        let _ = tx
                            .send(IPCCommand::SetWallpaper {
                                path: entry,
                                wayland_display: None,
                                hyprland_instance: None,
                                transition: Some(trans),
                                duration: Some(dur),
                            })
                            .await;
                        IPCResponse::Success("Reverted to previous wallpaper".to_string())
                    } else {
                        IPCResponse::Error("Not enough history".to_string())
                    }
                }
                Ok(IPCCommand::SetPywal(enabled)) => {
                    let mut s = state.lock().await;
                    s.pywal_enabled = enabled;
                    s.save();
                    IPCResponse::Success(format!("Pywal set to {}", enabled))
                }
                Ok(IPCCommand::SetDefaultTransition { transition, duration }) => {
                    let mut s = state.lock().await;
                    s.default_transition = transition;
                    s.default_duration = duration;
                    s.save();
                    IPCResponse::Success("Default transition updated".to_string())
                }
                Ok(IPCCommand::SetWallpaperDir(dir)) => {
                    let mut s = state.lock().await;
                    let path = PathBuf::from(dir);
                    if path.exists() && path.is_dir() {
                        s.wallpapers_dir = path;
                        s.save();
                        IPCResponse::Success(format!(
                            "Wallpaper directory updated to: {:?}",
                            s.wallpapers_dir
                        ))
                    } else {
                        IPCResponse::Error("Directory does not exist".to_string())
                    }
                }
                Ok(IPCCommand::GetWallpapersDir) => {
                    let s = state.lock().await;
                    IPCResponse::WallpaperDir(s.wallpapers_dir.to_string_lossy().to_string())
                }
                Ok(IPCCommand::ScheduleSet { time, path }) => {
                    let mut s = state.lock().await;
                    s.schedule.push(common::ScheduleEntry { time, path });
                    s.save();
                    IPCResponse::Success("Schedule entry added".to_string())
                }
                Ok(IPCCommand::GetSchedule) => {
                    let s = state.lock().await;
                    IPCResponse::Schedule {
                        enabled: s.schedule_enabled,
                        schedule: s.schedule.clone(),
                    }
                }
                Ok(IPCCommand::ToggleSchedule) => {
                    let mut s = state.lock().await;
                    s.schedule_enabled = !s.schedule_enabled;
                    s.save();
                    IPCResponse::Success(format!(
                        "Schedule {}",
                        if s.schedule_enabled { "enabled" } else { "disabled" }
                    ))
                }
                Ok(_) => IPCResponse::Success("Acknowledged".to_string()),
                Err(e) => IPCResponse::Error(format!("Invalid command: {}", e)),
            };
            let response_bytes = serde_json::to_vec(&response).unwrap();
            let _ = stream.write_all(&response_bytes).await;
            let _ = stream.shutdown().await;
        }
        _ => {}
    }
}
