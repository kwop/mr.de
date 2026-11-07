//! Before/after screenshots: runs the user's screenshot command in both worktrees, then
//! compares PNG files with the same relative name pixel by pixel.
use anyhow::{Context, bail};
use mrde_core::insights::{VisualReport, VisualScreen};
use std::{
    io::BufWriter,
    path::{Path, PathBuf},
    time::Duration,
};
use tokio::process::Command;

/// A channel difference below this is antialiasing noise, not a change.
const TOLERANCE: i16 = 16;

/// Runs the configured command in `worktree`; it must write PNG files to
/// `$MRDE_SCREENSHOTS_DIR`. The command comes from the user's configuration, never from
/// the repository or the MR.
pub async fn capture(
    command: &str,
    worktree: &Path,
    output: &Path,
    side: &str,
) -> anyhow::Result<()> {
    if output.exists() {
        std::fs::remove_dir_all(output).context("Cannot clear previous screenshots")?;
    }
    std::fs::create_dir_all(output).context("Cannot create the screenshot folder")?;
    let result = tokio::time::timeout(
        Duration::from_secs(20 * 60),
        Command::new("/bin/sh")
            .arg("-c")
            .arg(command)
            .current_dir(worktree)
            .env("MRDE_SCREENSHOTS_DIR", output)
            .env("MRDE_SIDE", side)
            .kill_on_drop(true)
            .output(),
    )
    .await
    .context("The screenshot command timed out after 20 minutes")??;
    if !result.status.success() {
        let stderr = String::from_utf8_lossy(&result.stderr);
        let tail: Vec<&str> = stderr.lines().rev().take(6).collect();
        bail!(
            "The screenshot command failed on the {side} commit ({}): {}",
            result.status,
            tail.into_iter().rev().collect::<Vec<_>>().join(" / ")
        );
    }
    Ok(())
}

fn pngs(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("png"))
            {
                out.push(
                    path.strip_prefix(root)
                        .map(Path::to_path_buf)
                        .unwrap_or(path),
                );
            }
        }
    }
    out.sort();
    out
}

struct Image {
    width: u32,
    height: u32,
    rgba: Vec<u8>,
}

fn decode(path: &Path) -> anyhow::Result<Image> {
    let mut decoder = png::Decoder::new(std::io::BufReader::new(std::fs::File::open(path)?));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info()?;
    let mut buffer = vec![0; reader.output_buffer_size().context("Image too large")?];
    let info = reader.next_frame(&mut buffer)?;
    buffer.truncate(info.buffer_size());
    let rgba = match info.color_type {
        png::ColorType::Rgba => buffer,
        png::ColorType::Rgb => buffer
            .chunks_exact(3)
            .flat_map(|p| [p[0], p[1], p[2], 255])
            .collect(),
        png::ColorType::GrayscaleAlpha => buffer
            .chunks_exact(2)
            .flat_map(|p| [p[0], p[0], p[0], p[1]])
            .collect(),
        png::ColorType::Grayscale => buffer.iter().flat_map(|g| [*g, *g, *g, 255]).collect(),
        png::ColorType::Indexed => bail!("Unexpanded indexed PNG"),
    };
    Ok(Image {
        width: info.width,
        height: info.height,
        rgba,
    })
}

/// Writes an RGBA image; used for synthetic screenshots in previews and tests.
pub fn write_rgba(path: &Path, width: u32, height: u32, rgba: Vec<u8>) -> anyhow::Result<()> {
    encode(
        path,
        &Image {
            width,
            height,
            rgba,
        },
    )
}

fn encode(path: &Path, image: &Image) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut encoder = png::Encoder::new(
        BufWriter::new(std::fs::File::create(path)?),
        image.width,
        image.height,
    );
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header()?.write_image_data(&image.rgba)?;
    Ok(())
}

/// Share of differing pixels, and a diff image: changes in red over the dimmed "after".
fn difference(before: &Image, after: &Image) -> (f64, Image) {
    let mut rgba = Vec::with_capacity(after.rgba.len());
    let mut changed = 0u64;
    for (b, a) in before.rgba.chunks_exact(4).zip(after.rgba.chunks_exact(4)) {
        let differs = b
            .iter()
            .zip(a)
            .any(|(x, y)| (*x as i16 - *y as i16).abs() > TOLERANCE);
        if differs {
            changed += 1;
            rgba.extend_from_slice(&[240, 72, 72, 255]);
        } else {
            rgba.extend(a[..3].iter().map(|c| (*c as f32 * 0.3) as u8));
            rgba.push(255);
        }
    }
    let total = (after.width as u64 * after.height as u64).max(1);
    (
        changed as f64 / total as f64,
        Image {
            width: after.width,
            height: after.height,
            rgba,
        },
    )
}

pub fn compare(
    before_dir: &Path,
    after_dir: &Path,
    diff_dir: &Path,
) -> anyhow::Result<VisualReport> {
    let mut names = pngs(before_dir);
    names.extend(pngs(after_dir));
    names.sort();
    names.dedup();
    let mut screens = Vec::new();
    for name in names {
        let (before, after) = (before_dir.join(&name), after_dir.join(&name));
        let mut screen = VisualScreen {
            name: name.with_extension("").display().to_string(),
            before: before.exists().then(|| before.clone()),
            after: after.exists().then(|| after.clone()),
            diff: None,
            changed: None,
        };
        if let (Some(before), Some(after)) = (&screen.before, &screen.after) {
            let (before, after) = (decode(before)?, decode(after)?);
            if (before.width, before.height) != (after.width, after.height) {
                screen.changed = Some(1.0);
            } else {
                let (ratio, image) = difference(&before, &after);
                let path = diff_dir.join(&name);
                encode(&path, &image)?;
                screen.diff = Some(path);
                screen.changed = Some(ratio);
            }
        }
        screens.push(screen);
    }
    screens.sort_by(|a, b| {
        b.changed
            .unwrap_or(2.0)
            .total_cmp(&a.changed.unwrap_or(2.0))
            .then_with(|| a.name.cmp(&b.name))
    });
    Ok(VisualReport { screens })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(path: &Path, width: u32, height: u32, color: [u8; 4], patch: Option<[u8; 4]>) {
        let mut rgba = Vec::new();
        for y in 0..height {
            for x in 0..width {
                let inside = x < width / 2 && y < height / 2;
                rgba.extend_from_slice(&match (inside, patch) {
                    (true, Some(c)) => c,
                    _ => color,
                });
            }
        }
        encode(
            path,
            &Image {
                width,
                height,
                rgba,
            },
        )
        .unwrap();
    }

    #[tokio::test]
    async fn runs_the_command_per_side_and_reports_changed_pixels() {
        let temp = tempfile::tempdir().unwrap();
        let (before, after, diff) = (
            temp.path().join("before"),
            temp.path().join("after"),
            temp.path().join("diff"),
        );
        let script = "printf '%s' \"$MRDE_SIDE\" > \"$MRDE_SCREENSHOTS_DIR/side.txt\"";
        capture(script, temp.path(), &before, "before")
            .await
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(before.join("side.txt")).unwrap(),
            "before"
        );
        assert!(
            capture("echo boom >&2; exit 3", temp.path(), &after, "after")
                .await
                .unwrap_err()
                .to_string()
                .contains("boom")
        );
        std::fs::create_dir_all(after.join("forms")).unwrap();
        std::fs::create_dir_all(before.join("forms")).unwrap();
        solid(
            &before.join("forms/series.png"),
            10,
            10,
            [20, 20, 20, 255],
            None,
        );
        solid(
            &after.join("forms/series.png"),
            10,
            10,
            [20, 20, 20, 255],
            Some([70, 200, 120, 255]),
        );
        solid(&before.join("same.png"), 4, 4, [1, 2, 3, 255], None);
        solid(&after.join("same.png"), 4, 4, [5, 6, 7, 255], None);
        solid(&after.join("new.png"), 4, 4, [1, 2, 3, 255], None);
        let report = compare(&before, &after, &diff).unwrap();
        let summary: Vec<_> = report
            .screens
            .iter()
            .map(|s| (s.name.as_str(), s.changed))
            .collect();
        assert_eq!(
            summary,
            [
                ("new", None),
                ("forms/series", Some(0.25)),
                ("same", Some(0.0))
            ]
        );
        assert!(diff.join("forms/series.png").exists());
    }
}
