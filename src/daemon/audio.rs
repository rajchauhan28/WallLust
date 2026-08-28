//! System-audio spectrum capture for scene wallpapers.
//!
//! Captures the default audio output's monitor with `parec` (ships with
//! PipeWire's PulseAudio shim), runs an FFT, and emits 8 log-spaced spectrum
//! bands (each 0..1, low -> high) with automatic gain so it self-adjusts to any
//! volume. No cava or other daemon required.

use rustfft::{num_complex::Complex, FftPlanner};
use std::io::Read;
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, Stdio};

pub const BANDS: usize = 8;

const SAMPLE_RATE: u32 = 44100;
const FFT_SIZE: usize = 1024;

/// Starts audio capture, invoking `on_bands` (~40x/second) with the current
/// spectrum. Returns the capture child process so the caller can keep it alive
/// and reap it; returns `None` if capture can't be started. `on_bands` returns
/// `Err` to stop capture (e.g. when the receiving end has gone away).
pub fn start<F>(mut on_bands: F) -> Option<Child>
where
    F: FnMut([f32; BANDS]) -> Result<(), ()> + Send + 'static,
{
    let device = default_monitor().unwrap_or_else(|| "@DEFAULT_MONITOR@".to_string());

    let mut cmd = Command::new("parec");
    cmd.args([
        "--format=s16le",
        &format!("--rate={SAMPLE_RATE}"),
        "--channels=1",
        "--latency-msec=25",
        "-d",
        &device,
    ])
    .stdout(Stdio::piped())
    .stderr(Stdio::null());

    // Ensure parec dies if the renderer process is killed (the daemon SIGKILLs
    // the renderer on backend handoff, which cannot run normal cleanup).
    unsafe {
        cmd.pre_exec(|| {
            libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL);
            Ok(())
        });
    }

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Audio capture: could not start parec ({e}); audio_* stays silent.");
            return None;
        }
    };

    let stdout = child.stdout.take()?;
    println!("Audio capture active on {device}");
    std::thread::spawn(move || run_capture(stdout, &mut on_bands));
    Some(child)
}

fn default_monitor() -> Option<String> {
    let out = Command::new("pactl").arg("get-default-sink").output().ok()?;
    if !out.status.success() {
        return None;
    }
    let sink = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if sink.is_empty() {
        None
    } else {
        Some(format!("{sink}.monitor"))
    }
}

fn run_capture<F>(mut stdout: impl Read, on_bands: &mut F)
where
    F: FnMut([f32; BANDS]) -> Result<(), ()>,
{
    let fft = FftPlanner::<f32>::new().plan_fft_forward(FFT_SIZE);
    let hann: Vec<f32> = (0..FFT_SIZE)
        .map(|n| {
            let x = std::f32::consts::PI * 2.0 * n as f32 / (FFT_SIZE as f32 - 1.0);
            0.5 - 0.5 * x.cos()
        })
        .collect();

    let mut bytes = vec![0u8; FFT_SIZE * 2]; // s16le = 2 bytes/sample
    let mut buf = vec![Complex::<f32>::new(0.0, 0.0); FFT_SIZE];
    let mut smooth = [0f32; BANDS];
    let mut peak = 1e-4f32;

    loop {
        if read_exact(&mut stdout, &mut bytes).is_err() {
            println!("Audio capture stopped (input ended).");
            return;
        }

        for i in 0..FFT_SIZE {
            let s = i16::from_le_bytes([bytes[2 * i], bytes[2 * i + 1]]) as f32 / 32768.0;
            buf[i] = Complex::new(s * hann[i], 0.0);
        }
        fft.process(&mut buf);

        let nyq = FFT_SIZE / 2;
        let mut bands = [0f32; BANDS];
        for (band, slot) in bands.iter_mut().enumerate() {
            let lo = band_edge(band, nyq);
            let hi = band_edge(band + 1, nyq).max(lo + 1);
            let mut sum = 0.0;
            for c in &buf[lo..hi] {
                sum += c.norm();
            }
            *slot = sum / (hi - lo) as f32;
        }

        // Auto-gain: normalise against a slowly decaying peak so any volume maps
        // into a usable 0..1 range.
        let cur_max = bands.iter().copied().fold(0.0f32, f32::max);
        peak = (peak * 0.995).max(cur_max).max(1e-4);

        for (band, slot) in smooth.iter_mut().enumerate() {
            let n = (bands[band] / peak).clamp(0.0, 1.0).powf(0.6);
            // Fast attack, slow release for a punchy but smooth response.
            *slot = if n > *slot {
                *slot * 0.4 + n * 0.6
            } else {
                *slot * 0.82 + n * 0.18
            };
        }

        if on_bands(smooth).is_err() {
            return;
        }
    }
}

/// Log-spaced bin edge for band `b` (0..=BANDS), from ~bin 2 to the Nyquist bin.
fn band_edge(b: usize, nyq_bins: usize) -> usize {
    let min_bin = 2.0f32;
    let max_bin = nyq_bins as f32;
    let t = b as f32 / BANDS as f32;
    (min_bin * (max_bin / min_bin).powf(t)).round() as usize
}

fn read_exact(r: &mut impl Read, buf: &mut [u8]) -> std::io::Result<()> {
    let mut filled = 0;
    while filled < buf.len() {
        match r.read(&mut buf[filled..])? {
            0 => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    "audio stream ended",
                ))
            }
            n => filled += n,
        }
    }
    Ok(())
}
