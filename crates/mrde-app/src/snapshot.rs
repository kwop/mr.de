//! Visual checks: `--ui-preview` with `MRDE_SNAPSHOT=out.png` saves a picture of the app's
//! own window, then quits. macOS lets a process capture its own windows without the
//! Screen Recording permission, so this works from a terminal or an agent.
use anyhow::{Context, bail};
use core_foundation::{number::CFNumber, string::CFString};
use core_graphics::{
    geometry::{CGPoint, CGRect, CGSize},
    window::{
        create_description_from_array, create_image, create_window_list, kCGNullWindowID,
        kCGWindowImageBoundsIgnoreFraming, kCGWindowListOptionIncludingWindow,
        kCGWindowListOptionOnScreenOnly,
    },
};
use gpui::App;
use std::{
    io::BufWriter,
    path::PathBuf,
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

/// Set by the real-application scenario once the requested screen is displayed.
pub static READY: AtomicBool = AtomicBool::new(false);

fn number(
    description: &core_foundation::dictionary::CFDictionary<
        CFString,
        core_foundation::base::CFType,
    >,
    key: &'static str,
) -> Option<i64> {
    description
        .find(CFString::from_static_string(key))
        .and_then(|value| value.downcast::<CFNumber>())
        .and_then(|value| value.to_i64())
}

/// The largest normal-layer window owned by this process.
fn own_window() -> Option<u32> {
    let ids = create_window_list(kCGWindowListOptionOnScreenOnly, kCGNullWindowID)?;
    let descriptions = create_description_from_array(ids)?;
    let pid = i64::from(std::process::id());
    descriptions
        .iter()
        .filter(|d| number(d, "kCGWindowOwnerPID") == Some(pid))
        .filter(|d| number(d, "kCGWindowLayer") == Some(0))
        .filter_map(|d| number(&d, "kCGWindowNumber"))
        .next()
        .and_then(|id| u32::try_from(id).ok())
}

pub fn capture(path: &std::path::Path) -> anyhow::Result<()> {
    let window = own_window().context("The app window is not on screen")?;
    // CGRectNull: the smallest rectangle enclosing the requested window.
    let null = CGRect::new(
        &CGPoint::new(f64::INFINITY, f64::INFINITY),
        &CGSize::new(0., 0.),
    );
    let image = create_image(
        null,
        kCGWindowListOptionIncludingWindow,
        window,
        kCGWindowImageBoundsIgnoreFraming,
    )
    .context("macOS refused to capture the window")?;
    let (width, height, stride) = (image.width(), image.height(), image.bytes_per_row());
    if image.bits_per_pixel() != 32 || width == 0 || height == 0 {
        bail!("Unsupported window image format");
    }
    let data = image.data();
    let bytes = data.bytes();
    // Window images are 32-bit little-endian BGRA.
    let mut rgba = Vec::with_capacity(width * height * 4);
    for row in 0..height {
        for pixel in bytes[row * stride..row * stride + width * 4].chunks_exact(4) {
            rgba.extend_from_slice(&[pixel[2], pixel[1], pixel[0], pixel[3]]);
        }
    }
    let file = std::fs::File::create(path).context("Cannot create the snapshot file")?;
    let mut encoder = png::Encoder::new(BufWriter::new(file), width as u32, height as u32);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header()?.write_image_data(&rgba)?;
    Ok(())
}

/// Requested by `MRDE_SNAPSHOT` in preview mode. Real data is captured only with the
/// explicit `MRDE_SNAPSHOT_REAL=1` opt-in.
pub fn requested() -> Option<PathBuf> {
    let preview = std::env::args().any(|arg| arg == "--ui-preview");
    (preview || real_opt_in())
        .then(|| std::env::var_os("MRDE_SNAPSHOT").map(PathBuf::from))
        .flatten()
}

fn real_opt_in() -> bool {
    std::env::var("MRDE_SNAPSHOT_REAL").is_ok_and(|value| value == "1")
}

/// A capture of the real application (not the preview) was explicitly requested.
pub fn real_requested() -> bool {
    !std::env::args().any(|arg| arg == "--ui-preview")
        && real_opt_in()
        && std::env::var_os("MRDE_SNAPSHOT").is_some()
}

pub fn schedule(path: PathBuf, cx: &mut App) {
    let delay = std::env::var("MRDE_SNAPSHOT_DELAY_MS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(2_500);
    let real = real_requested();
    cx.spawn(async move |cx| {
        // Real data arrives from GitLab: wait for the scenario (at most three minutes),
        // then let images and layout settle for the configured delay.
        let started = Instant::now();
        while real && !READY.load(Ordering::SeqCst) && started.elapsed() < Duration::from_secs(180)
        {
            cx.background_executor()
                .timer(Duration::from_millis(250))
                .await;
        }
        cx.background_executor()
            .timer(Duration::from_millis(delay))
            .await;
        match capture(&path) {
            Ok(()) => eprintln!("Snapshot saved to {}", path.display()),
            Err(error) => eprintln!("Snapshot failed: {error:#}"),
        }
        let _ = cx.update(|cx| cx.quit());
    })
    .detach();
}

/// `MRDE_WINDOW=1280x860` sets the preview window size for screenshots.
pub fn window_size() -> Option<(f32, f32)> {
    let value = std::env::var("MRDE_WINDOW").ok()?;
    let (width, height) = value.split_once('x')?;
    Some((width.parse().ok()?, height.parse().ok()?))
}
