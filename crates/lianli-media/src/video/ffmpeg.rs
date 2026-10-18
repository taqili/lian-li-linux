use super::process::PROBE_TIMEOUT;
use crate::common::MediaError;
use crate::PreparationControl;
use lianli_shared::media::MediaFraming;
use lianli_shared::template::ImageFit;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::SystemTime;

/// Probe results keyed by path and invalidated when the file's size or
/// modification time changes, so replaying a playlist does not re-run ffprobe.
#[derive(Clone, Copy, Default)]
struct Probed {
    fps: Option<Option<f32>>,
    size: Option<(u32, u32)>,
}

type Stamp = (u64, SystemTime);
const MAX_PROBES: usize = 512;
static PROBES: parking_lot::Mutex<Option<std::collections::HashMap<PathBuf, (Stamp, Probed)>>> =
    parking_lot::Mutex::new(None);

fn stamp(path: &Path) -> Option<Stamp> {
    let metadata = std::fs::metadata(path).ok()?;
    Some((metadata.len(), metadata.modified().ok()?))
}

fn cached_probe(path: &Path) -> Option<Probed> {
    let stamp = stamp(path)?;
    let probes = PROBES.lock();
    let (cached, probed) = probes.as_ref()?.get(path)?;
    (*cached == stamp).then_some(*probed)
}

fn remember_probe(path: &Path, update: impl FnOnce(&mut Probed)) {
    let Some(stamp) = stamp(path) else {
        return;
    };
    let mut probes = PROBES.lock();
    let probes = probes.get_or_insert_with(Default::default);
    if probes.len() >= MAX_PROBES && !probes.contains_key(path) {
        probes.clear();
    }
    let entry = probes
        .entry(path.to_path_buf())
        .or_insert((stamp, Probed::default()));
    if entry.0 != stamp {
        *entry = (stamp, Probed::default());
    }
    update(&mut entry.1);
}

/// Probe the source's average frame rate via ffprobe. Returns `None` if the
/// file isn't a video/animated source or ffprobe is unavailable.
pub fn probe_source_fps(path: &Path) -> Option<f32> {
    probe_fps(path, &PreparationControl::new(false))
        .ok()
        .flatten()
}

fn probe_fps(path: &Path, control: &PreparationControl) -> Result<Option<f32>, MediaError> {
    if let Some(fps) = cached_probe(path).and_then(|probed| probed.fps) {
        return Ok(fps);
    }
    let fps = run_probe_fps(path, control)?;
    remember_probe(path, |probed| probed.fps = Some(fps));
    Ok(fps)
}

fn run_probe_fps(path: &Path, control: &PreparationControl) -> Result<Option<f32>, MediaError> {
    let mut cmd = Command::new("ffprobe");
    cmd.args([
        "-v",
        "error",
        "-select_streams",
        "v:0",
        "-show_entries",
        "stream=avg_frame_rate",
        "-of",
        "default=noprint_wrappers=1:nokey=1",
    ])
    .arg(path);
    let output = match control.output(cmd, PROBE_TIMEOUT) {
        Ok(output) => output,
        Err(MediaError::Cancelled) => return Err(MediaError::Cancelled),
        Err(err) => {
            tracing::warn!("probing source fps failed: {err}");
            control.check()?;
            return Ok(None);
        }
    };
    if !output.status.success() {
        return Ok(None);
    }
    let s = String::from_utf8_lossy(&output.stdout);
    let s = s.trim();
    Ok(s.split_once('/').and_then(|(num, den)| {
        let num: f32 = num.parse().ok()?;
        let den: f32 = den.parse().ok()?;
        (num.is_finite() && den.is_finite() && den > 0.0 && num > 0.0).then_some(num / den)
    }))
}

/// Display size of the first video stream, after the rotation that ffmpeg
/// applies automatically when decoding.
pub(crate) fn probe_display_size(
    path: &Path,
    control: &PreparationControl,
) -> Result<(u32, u32), MediaError> {
    if let Some(size) = cached_probe(path).and_then(|probed| probed.size) {
        return Ok(size);
    }
    let size = run_probe_display_size(path, control)?;
    remember_probe(path, |probed| probed.size = Some(size));
    Ok(size)
}

fn run_probe_display_size(
    path: &Path,
    control: &PreparationControl,
) -> Result<(u32, u32), MediaError> {
    let mut cmd = Command::new("ffprobe");
    cmd.args([
        "-v",
        "error",
        "-select_streams",
        "v:0",
        "-show_entries",
        "stream=width,height:stream_tags=rotate:stream_side_data=rotation",
        "-of",
        "default=noprint_wrappers=1",
    ])
    .arg(path);
    let output = control.output(cmd, PROBE_TIMEOUT)?;
    if !output.status.success() {
        return Err(MediaError::Ffmpeg(format!(
            "ffprobe could not read '{}': {}",
            path.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    parse_display_size(&String::from_utf8_lossy(&output.stdout))
        .ok_or_else(|| MediaError::Ffmpeg(format!("'{}' has no video stream", path.display())))
}

fn parse_display_size(probe: &str) -> Option<(u32, u32)> {
    let mut width = None;
    let mut height = None;
    let mut quarter_turn = false;
    for line in probe.lines() {
        let Some((key, value)) = line.trim().split_once('=') else {
            continue;
        };
        match key {
            "width" => width = value.parse::<u32>().ok().filter(|&w| w > 0),
            "height" => height = value.parse::<u32>().ok().filter(|&h| h > 0),
            "rotation" | "TAG:rotate" => {
                quarter_turn |= value
                    .parse::<f64>()
                    .is_ok_and(|degrees| (degrees.abs().round() as i64) % 180 == 90);
            }
            _ => {}
        }
    }
    let (width, height) = (width?, height?);
    Some(if quarter_turn {
        (height, width)
    } else {
        (width, height)
    })
}

pub(crate) fn cap_fps_cancellable(
    path: &Path,
    target: f32,
    control: &PreparationControl,
) -> Result<f32, MediaError> {
    control.check()?;
    let target = target.max(1.0);
    Ok(match probe_fps(path, control)? {
        Some(source) if source >= 1.0 => target.min(source),
        _ => target,
    })
}

/// Cap `target` by the source's native fps (when probeable). Always at least 1.
pub fn cap_fps_to_source(path: &Path, target: f32) -> f32 {
    let target = target.max(1.0);
    match probe_source_fps(path) {
        Some(src) if src >= 1.0 => target.min(src),
        _ => target,
    }
}

pub(super) fn stream_rgba(
    input: &Path,
    fps: f32,
    width: u32,
    height: u32,
    framing: &MediaFraming,
    control: &PreparationControl,
    consume: impl FnMut(&[u8]) -> Result<(), MediaError>,
) -> Result<(), MediaError> {
    if !fps.is_finite() || fps < 1.0 {
        return Err(MediaError::InvalidFps);
    }
    let frame_bytes = super::frame_budget::rgba_bytes(width, height)?;
    let command = filtered_rgba_command(
        input,
        fps,
        &framing_filter(framing, width, height),
        false,
        control.hardware_video,
        None,
    );
    let output = control.stream_frames(command, frame_bytes, consume)?;
    check_rgba_output(output)
}

pub(super) fn check_rgba_output(output: std::process::Output) -> Result<(), MediaError> {
    if !output.status.success() {
        return Err(MediaError::Ffmpeg(format!(
            "ffmpeg exited with status {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(())
}

/// ffmpeg equivalent of `common::frame_rgb` for LCD video and H.264 paths.
pub(super) fn framing_filter(framing: &MediaFraming, width: u32, height: u32) -> String {
    let framing = framing.sanitized();
    let scale = format!("scale={width}:{height}:flags=lanczos");
    if framing.is_default() {
        return scale;
    }
    let zoom = framing.zoom;
    let (crop_w, crop_h) = if framing.fit == ImageFit::Cover {
        (
            format!("min(iw\\,ih*{width}/{height})/{zoom}"),
            format!("min(ih\\,iw*{height}/{width})/{zoom}"),
        )
    } else {
        (format!("iw/{zoom}"), format!("ih/{zoom}"))
    };
    let crop = format!(
        "crop=w={crop_w}:h={crop_h}:x=(iw-ow)/2*(1+({})):y=(ih-oh)/2*(1+({}))",
        framing.offset_x, framing.offset_y
    );
    let place = if framing.fit == ImageFit::Contain {
        format!("scale={width}:{height}:force_original_aspect_ratio=decrease:flags=lanczos,pad={width}:{height}:(ow-iw)/2:(oh-ih)/2:color=black")
    } else {
        scale
    };
    format!("{crop},{place}")
}

pub(super) fn rgba_command(
    input: &Path,
    fps: f32,
    size: (u32, u32),
    fit: ImageFit,
    looping: bool,
    hardware: bool,
    trim: Option<(f32, Option<f32>)>,
) -> Command {
    let (width, height) = size;
    let scale = match fit {
        ImageFit::Stretch => format!("scale={width}:{height}:flags=lanczos"),
        ImageFit::Contain => format!("scale={width}:{height}:force_original_aspect_ratio=decrease:flags=lanczos,format=rgba,pad={width}:{height}:(ow-iw)/2:(oh-ih)/2:color=black@0"),
        ImageFit::Cover => format!("scale={width}:{height}:force_original_aspect_ratio=increase:flags=lanczos,crop={width}:{height}"),
    };
    filtered_rgba_command(input, fps, &scale, looping, hardware, trim)
}

fn filtered_rgba_command(
    input: &Path,
    fps: f32,
    filter: &str,
    looping: bool,
    hardware: bool,
    trim: Option<(f32, Option<f32>)>,
) -> Command {
    let mut command = Command::new("ffmpeg");
    command.args([
        "-hide_banner",
        "-nostdin",
        "-loglevel",
        "error",
        "-max_alloc",
        "67108864",
        "-cpucount",
        "1",
        "-filter_threads",
        "1",
        "-filter_complex_threads",
        "1",
        "-threads",
        "1",
        "-hwaccel",
        if hardware { "auto" } else { "none" },
        "-max_pixels",
        "16777216",
    ]);
    if looping {
        command.args(["-stream_loop", "-1"]);
    }
    if let Some((start, _)) = trim.filter(|(start, _)| *start > 0.0) {
        command.args(["-ss", &format!("{start:.3}")]);
    }
    if let Some(length) = trim.and_then(|(start, end)| end.map(|end| end - start)) {
        command.args(["-t", &format!("{:.3}", length.max(0.001))]);
    }
    command.arg("-i").arg(input).args([
        "-map",
        "0:v:0",
        "-an",
        "-sn",
        "-dn",
        "-vf",
        filter,
        "-r",
        &fps.to_string(),
        "-pix_fmt",
        "rgba",
        "-threads",
        "1",
        "-c:v",
        "rawvideo",
        "-f",
        "rawvideo",
        "pipe:1",
    ]);
    command
}

#[cfg(test)]
mod tests {
    use super::parse_display_size;
    use crate::PreparationControl;
    use std::process::Command;

    #[test]
    fn quarter_turn_rotation_swaps_reported_dimensions() {
        assert_eq!(
            parse_display_size("width=320\nheight=180\n"),
            Some((320, 180))
        );
        assert_eq!(
            parse_display_size("width=320\nheight=180\nrotation=90\n"),
            Some((180, 320))
        );
        assert_eq!(
            parse_display_size("width=1920\nheight=1080\nTAG:rotate=270\n"),
            Some((1080, 1920))
        );
        assert_eq!(
            parse_display_size("width=1920\nheight=1080\nrotation=-180\n"),
            Some((1920, 1080))
        );
        assert_eq!(parse_display_size("width=0\nheight=1080\n"), None);
        assert_eq!(parse_display_size(""), None);
    }

    #[test]
    fn cached_probe_is_reused_until_the_file_changes() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("clip.mp4");
        let encode = |size: &str| {
            let status = Command::new("ffmpeg")
                .args(["-v", "error", "-y", "-f", "lavfi", "-i"])
                .arg(format!("testsrc=size={size}:rate=5"))
                .args(["-t", "0.4", "-pix_fmt", "yuv420p"])
                .arg(&path)
                .status()
                .unwrap();
            assert!(status.success());
        };
        let control = PreparationControl::new(false);
        encode("32x16");
        assert_eq!(
            super::probe_display_size(&path, &control).unwrap(),
            (32, 16)
        );
        assert_eq!(
            super::cached_probe(&path).and_then(|probed| probed.size),
            Some((32, 16))
        );
        encode("48x16");
        assert_eq!(super::cached_probe(&path).map(|probed| probed.size), None);
        assert_eq!(
            super::probe_display_size(&path, &control).unwrap(),
            (48, 16)
        );
    }
}
