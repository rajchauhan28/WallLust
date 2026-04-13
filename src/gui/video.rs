use ffmpeg_next as ffmpeg;
use slint::SharedPixelBuffer;
use slint::Rgb8Pixel;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

pub fn spawn_video_player(
    handle: slint::Weak<crate::AppWindow>,
    path: String,
    cancel_flag: Arc<AtomicBool>,
) -> anyhow::Result<()> {
    let _ = ffmpeg::init();
    
    let mut ictx = ffmpeg::format::input(&path)?;
    let input = ictx
        .streams()
        .best(ffmpeg::media::Type::Video)
        .ok_or(ffmpeg::Error::StreamNotFound)?;
        
    let video_stream_index = input.index();
    let time_base = input.time_base();
    
    let context_decoder = ffmpeg::codec::context::Context::from_parameters(input.parameters())?;
    let mut decoder = context_decoder.decoder().video()?;
    
    let mut scaler = ffmpeg::software::scaling::context::Context::get(
        decoder.format(),
        decoder.width(),
        decoder.height(),
        ffmpeg::format::Pixel::RGB24,
        decoder.width(),
        decoder.height(),
        ffmpeg::software::scaling::flag::Flags::BILINEAR,
    )?;

    let mut frame = ffmpeg::frame::Video::empty();
    let mut rgb_frame = ffmpeg::frame::Video::empty();

    let mut start_time = Instant::now();
    let mut first_pts = None;

    while !cancel_flag.load(Ordering::Relaxed) {
        let mut got_packet = false;
        
        for (stream, packet) in ictx.packets() {
            if cancel_flag.load(Ordering::Relaxed) { break; }
            if stream.index() == video_stream_index {
                got_packet = true;
                
                let _ = decoder.send_packet(&packet);
                
                while decoder.receive_frame(&mut frame).is_ok() {
                    if cancel_flag.load(Ordering::Relaxed) { break; }
                    
                    if scaler.run(&frame, &mut rgb_frame).is_ok() {
                        let mut pixel_buffer = SharedPixelBuffer::<Rgb8Pixel>::new(rgb_frame.width(), rgb_frame.height());
                        let ffmpeg_line_iter = rgb_frame.data(0).chunks_exact(rgb_frame.stride(0));
                        let slint_pixel_line_iter = pixel_buffer
                            .make_mut_bytes()
                            .chunks_mut(rgb_frame.width() as usize * core::mem::size_of::<Rgb8Pixel>());

                        for (source_line, dest_line) in ffmpeg_line_iter.zip(slint_pixel_line_iter) {
                            dest_line.copy_from_slice(&source_line[..dest_line.len()])
                        }
                        
                        let handle_copy = handle.clone();
                        let _ = slint::invoke_from_event_loop(move || {
                            let img = slint::Image::from_rgb8(pixel_buffer);
                            if let Some(h) = handle_copy.upgrade() {
                                h.set_preview_image(img);
                            }
                        });

                        // Sync video timing
                        let pts = frame.pts().unwrap_or(0);
                        if first_pts.is_none() {
                            first_pts = Some(pts);
                            start_time = Instant::now();
                        } else if let Some(first) = first_pts {
                            let pts_diff = pts - first;
                            let target_duration = Duration::from_secs_f64((pts_diff as f64 * time_base.numerator() as f64) / time_base.denominator() as f64);
                            let elapsed = start_time.elapsed();
                            if target_duration > elapsed {
                                std::thread::sleep(target_duration - elapsed);
                            }
                        }
                    }
                }
            }
        }
        
        if !got_packet {
            // EOF, restart
            let _ = ictx.seek(0, ..);
            first_pts = None;
            start_time = Instant::now();
        }
    }
    
    Ok(())
}
