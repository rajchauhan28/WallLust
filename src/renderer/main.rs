//! WebKit-based renderer for "web scenes": a directory containing `scene.json`
//! authored in the browser editor. The scene is drawn with plain DOM (no
//! external library, no network) and driven live with the same property
//! contract as Slint scenes — cursor position and an 8-band audio spectrum —
//! pushed into the page from this process.

use gtk4::gio;
use gtk4::glib;
use gtk4::prelude::*;
use gtk4::{Application, ApplicationWindow};
use gtk4_layer_shell::{Edge, Layer, LayerShell};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use webkit6::prelude::*;
use webkit6::WebView;

// The audio capture module is shared with the daemon; it depends only on
// rustfft/libc/std, so it compiles standalone into this binary too.
#[path = "../daemon/audio.rs"]
mod audio;

/// Live inputs pushed into the page ~30x/second.
struct LiveState {
    cursor_x: f64,
    cursor_y: f64,
    bands: [f32; audio::BANDS],
}

/// The page shell. `__SCENE_JSON__` is replaced with the (escaped) scene.json.
/// Rendering is declarative DOM; `window.__wallInput` is called from Rust to
/// deliver cursor + audio, and a requestAnimationFrame loop applies subtle
/// cursor parallax plus audio-reactive pulsing to `audioReactive` objects.
const PAGE: &str = r#"<!DOCTYPE html>
<html>
<head>
<style>
  html, body { margin: 0; padding: 0; width: 100vw; height: 100vh; overflow: hidden; background: #000; }
  #stage { position: absolute; left: 0; top: 0; width: 1920px; height: 1080px; transform-origin: 0 0; }
  .obj { position: absolute; will-change: transform; }
</style>
</head>
<body>
<div id="stage"></div>
<script>
const scene = __SCENE_JSON__;
const DW = 1920, DH = 1080;
const stage = document.getElementById('stage');
document.body.style.background = scene.bg_color || '#000';

function layout() {
  const s = Math.max(window.innerWidth / DW, window.innerHeight / DH);
  const ox = (window.innerWidth - DW * s) / 2;
  const oy = (window.innerHeight - DH * s) / 2;
  stage.style.transform = `translate(${ox}px, ${oy}px) scale(${s})`;
  return { s, ox, oy };
}
window.addEventListener('resize', layout);
layout();

const els = [];
(scene.objects || []).forEach((o) => {
  let el;
  if (o.type === 'media' && scene.has_media && scene.media_filename) {
    const url = scene.media_filename;
    if (/\.(mp4|webm|mkv|mov)$/i.test(url)) {
      el = document.createElement('video');
      el.src = url; el.loop = true; el.muted = true; el.autoplay = true; el.playsInline = true;
      el.play().catch(() => {});
    } else {
      el = document.createElement('img');
      el.src = url;
    }
    el.style.objectFit = 'cover';
  } else {
    el = document.createElement('div');
    el.style.background = o.fill || '#888888';
  }
  el.className = 'obj';
  el.style.left = (o.left || 0) + 'px';
  el.style.top = (o.top || 0) + 'px';
  el.style.width = (o.width || 100) + 'px';
  el.style.height = (o.height || 100) + 'px';
  el.style.opacity = (o.opacity == null ? 1 : o.opacity);
  stage.appendChild(el);
  els.push({ el, o, angle: o.angle || 0 });
});

// Contract exposed to any custom scene JS, and fed by the renderer process.
window.walllust = { time_ms: 0, time_s: 0, cursor_x: DW / 2, cursor_y: DH / 2, audio: [0,0,0,0,0,0,0,0] };
window.__wallInput = function (cx, cy, bands) {
  // cx/cy arrive as global screen pixels; map them into 1920x1080 design space.
  const s = Math.max(window.innerWidth / DW, window.innerHeight / DH);
  const ox = (window.innerWidth - DW * s) / 2;
  const oy = (window.innerHeight - DH * s) / 2;
  window.walllust.cursor_x = (cx - ox) / s;
  window.walllust.cursor_y = (cy - oy) / s;
  window.walllust.audio = bands;
};

const t0 = performance.now();
function frame(now) {
  const w = window.walllust;
  w.time_ms = now - t0;
  w.time_s = w.time_ms / 1000;
  const cxN = (w.cursor_x / DW - 0.5);
  const cyN = (w.cursor_y / DH - 0.5);
  els.forEach((r, i) => {
    const depth = ((i % 5) + 1) / 5;         // vary parallax by stacking order
    const px = cxN * 24 * depth;             // subtle cursor parallax (design px)
    const py = cyN * 24 * depth;
    let sc = 1;
    if (r.o.audioReactive) {
      const band = w.audio[i % 8] || 0;
      sc = 1 + band * 0.18;                  // pulse with its spectrum band
    }
    r.el.style.transform = `translate(${px}px, ${py}px) rotate(${r.angle}deg) scale(${sc})`;
  });
  requestAnimationFrame(frame);
}
requestAnimationFrame(frame);
</script>
</body>
</html>"#;

fn query_cursor() -> Option<(f64, f64)> {
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

fn main() {
    // Parse the scene directory ourselves rather than letting GApplication treat
    // it as a file to open (which, without HANDLES_OPEN, aborts with "this
    // application can not open files").
    let args: Vec<String> = std::env::args().collect();
    let scene_dir = if args.len() > 1 {
        std::path::PathBuf::from(&args[1])
    } else {
        dirs::config_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("."))
            .join("walllust/live_scenes/pro_editor_scene")
    };

    let app = Application::builder()
        .application_id("com.github.rajchauhan28.walllust.renderer")
        .build();

    app.connect_activate(move |app| {
        let scene_dir = scene_dir.clone();
        let scene_json_path = scene_dir.join("scene.json");
        let raw_json =
            std::fs::read_to_string(&scene_json_path).unwrap_or_else(|_| "{}".to_string());
        // Escape `<`/`>` as JSON unicode escapes: the value is inlined into an
        // inline <script>, and this keeps a `</script>` inside any string field
        // from breaking out of the tag while remaining valid JSON.
        let scene_json = raw_json.replace('<', "\\u003c").replace('>', "\\u003e");

        let window = ApplicationWindow::builder()
            .application(app)
            .title("walllust-renderer")
            .build();

        // Pin to the Wayland Background layer, full output, reserving no space
        // and rendering under panels' exclusive zones (no seam).
        window.init_layer_shell();
        window.set_layer(Layer::Background);
        window.set_exclusive_zone(-1);
        window.set_anchor(Edge::Top, true);
        window.set_anchor(Edge::Bottom, true);
        window.set_anchor(Edge::Left, true);
        window.set_anchor(Edge::Right, true);

        let webview = WebView::new();
        webview.set_background_color(&gtk4::gdk::RGBA::new(0.0, 0.0, 0.0, 0.0));

        let html = PAGE.replace("__SCENE_JSON__", &scene_json);
        let base_uri = format!("file://{}/", scene_dir.to_string_lossy());
        webview.load_html(&html, Some(&base_uri));

        window.set_child(Some(&webview));
        window.present();

        // --- Live inputs: cursor + audio, pushed into the page. ---
        let state = Arc::new(Mutex::new(LiveState {
            cursor_x: 960.0,
            cursor_y: 540.0,
            bands: [0.0; audio::BANDS],
        }));

        // Poll Hyprland for the global cursor (works even under windows).
        {
            let st = state.clone();
            std::thread::spawn(move || loop {
                if let Some((x, y)) = query_cursor() {
                    if let Ok(mut s) = st.lock() {
                        s.cursor_x = x;
                        s.cursor_y = y;
                    }
                }
                std::thread::sleep(Duration::from_millis(33));
            });
        }

        // Only capture audio if the scene actually uses it, so idle web scenes
        // don't spawn parec.
        let wants_audio = raw_json.contains("\"audioReactive\": true")
            || raw_json.contains("\"audioReactive\":true");
        let audio_child = if wants_audio {
            let st = state.clone();
            audio::start(move |bands| {
                if let Ok(mut s) = st.lock() {
                    s.bands = bands;
                }
                Ok(())
            })
        } else {
            None
        };

        // Push the latest inputs into the page ~30x/second. evaluate_javascript
        // must run on the GTK main thread, which this timer does.
        let wv = webview.clone();
        let st = state.clone();
        glib::timeout_add_local(Duration::from_millis(33), move || {
            // Keep the audio capture child alive for the lifetime of the timer.
            let _keep = &audio_child;
            let (cx, cy, bands) = {
                let s = st.lock().unwrap();
                (s.cursor_x, s.cursor_y, s.bands)
            };
            let bands_js = bands
                .iter()
                .map(|v| format!("{v:.4}"))
                .collect::<Vec<_>>()
                .join(",");
            let js = format!("window.__wallInput&&window.__wallInput({cx},{cy},[{bands_js}]);");
            wv.evaluate_javascript(&js, None, None, None::<&gio::Cancellable>, |_| {});
            glib::ControlFlow::Continue
        });
    });

    // Run with no file arguments so GApplication doesn't try to "open" the
    // scene directory we already parsed above.
    app.run_with_args(&["walllust-renderer"]);
}
