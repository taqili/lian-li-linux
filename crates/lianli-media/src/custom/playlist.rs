//! Mixed image, GIF and video playlist rendered through `CustomAsset`, with an
//! optional CPU/GPU temperature readout in the bars Fit mode leaves empty.

use super::helpers::{fast_overlay, fit_image, resolve_sensor_source};
use crate::common::get_exact_text_metrics;
use crate::resource_budget::RetainedBudget;
use crate::video::widget_stream::VideoStream;
use crate::{MediaError, PreparationControl};
use ab_glyph::{FontVec, PxScale};
use image::{Rgba, RgbaImage};
use lianli_shared::media::SensorSourceConfig;
use lianli_shared::playlist::{PlaylistConfig, PlaylistItemKind, TemperatureOverlay};
use lianli_shared::sensors::{
    pick_source_for_category, read_sensor_value, ResolvedSensor, SensorCategory, SensorInfo,
};
use lianli_shared::template::ImageFit;
use std::io::BufReader;
use std::path::PathBuf;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use tracing::{debug, warn};

/// Narrowest bar that still fits a legible two-line readout. A 16:9 clip on a
/// 1472×720 panel leaves 96px on each side.
const MIN_BAR: u32 = 64;
const SAMPLE_INTERVAL: Duration = Duration::from_secs(2);
const FAILURE_BACKOFF: Duration = Duration::from_secs(30);
const MAX_FPS: f32 = 60.0;
const UNAVAILABLE: &str = "--°C";
const LABEL_COLOR: Rgba<u8> = Rgba([170, 170, 170, 255]);
const VALUE_COLOR: Rgba<u8> = Rgba([255, 255, 255, 255]);
const BAR_COLOR: Rgba<u8> = Rgba([0, 0, 0, 255]);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Area {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

/// Where a source of `source` pixels lands on the canvas. Contain keeps the
/// aspect ratio inside the canvas; Cover and Stretch fill it.
pub(crate) fn place(source: (u32, u32), canvas: (u32, u32), fit: ImageFit) -> Area {
    let (cw, ch) = (canvas.0.max(1), canvas.1.max(1));
    if fit != ImageFit::Contain {
        return Area {
            x: 0,
            y: 0,
            width: cw,
            height: ch,
        };
    }
    let (sw, sh) = (f64::from(source.0.max(1)), f64::from(source.1.max(1)));
    let scale = (f64::from(cw) / sw).min(f64::from(ch) / sh);
    let width = ((sw * scale).round() as u32).clamp(1, cw);
    let height = ((sh * scale).round() as u32).clamp(1, ch);
    Area {
        x: (cw - width) / 2,
        y: (ch - height) / 2,
        width,
        height,
    }
}

/// The pair of empty bars beside or above and below the media: left/top for
/// the CPU readout and right/bottom for the GPU readout. `None` when neither
/// pair is at least `MIN_BAR` thick.
pub(crate) fn side_bars(media: Area, canvas: (u32, u32)) -> Option<[Area; 2]> {
    let (cw, ch) = canvas;
    let right = cw.saturating_sub(media.x + media.width);
    let bottom = ch.saturating_sub(media.y + media.height);
    let beside = media.x.min(right);
    let around = media.y.min(bottom);
    if beside >= MIN_BAR && beside >= around {
        Some([
            Area {
                x: 0,
                y: 0,
                width: media.x,
                height: ch,
            },
            Area {
                x: media.x + media.width,
                y: 0,
                width: right,
                height: ch,
            },
        ])
    } else if around >= MIN_BAR {
        Some([
            Area {
                x: 0,
                y: 0,
                width: cw,
                height: media.y,
            },
            Area {
                x: 0,
                y: media.y + media.height,
                width: cw,
                height: bottom,
            },
        ])
    } else {
        None
    }
}

struct Entry {
    path: PathBuf,
    kind: PlaylistItemKind,
}

enum Content {
    Still {
        pixels: RgbaImage,
        _budget: RetainedBudget,
    },
    Motion(VideoStream),
}

struct Loaded {
    area: Area,
    content: Content,
}

#[derive(Clone, Copy)]
struct Settings {
    canvas: (u32, u32),
    fit: ImageFit,
    fps: f32,
}

fn content_fit(fit: ImageFit) -> ImageFit {
    match fit {
        ImageFit::Contain => ImageFit::Stretch,
        other => other,
    }
}

fn animation_size(path: &std::path::Path) -> Result<(u32, u32), MediaError> {
    let file = BufReader::new(crate::image::open_image_file(path)?);
    let reader = image::ImageReader::new(file).with_guessed_format()?;
    Ok(reader.into_dimensions()?)
}

fn load(
    entry: &Entry,
    settings: &Settings,
    control: &PreparationControl,
) -> Result<Loaded, MediaError> {
    control.check()?;
    match entry.kind {
        PlaylistItemKind::Image => {
            let image = crate::image::open_image(&entry.path)?;
            control.check()?;
            let area = place(
                (image.width(), image.height()),
                settings.canvas,
                settings.fit,
            );
            let mut budget = RetainedBudget::default();
            budget.reserve(area.width as usize * area.height as usize * 4)?;
            let pixels = fit_image(image, area.width, area.height, content_fit(settings.fit));
            Ok(Loaded {
                area,
                content: Content::Still {
                    pixels,
                    _budget: budget,
                },
            })
        }
        PlaylistItemKind::Animation | PlaylistItemKind::Video => {
            let (source, fps) = if entry.kind == PlaylistItemKind::Animation {
                (animation_size(&entry.path)?, settings.fps)
            } else {
                (
                    crate::video::ffmpeg::probe_display_size(&entry.path, control)?,
                    crate::video::ffmpeg::cap_fps_cancellable(&entry.path, settings.fps, control)?,
                )
            };
            let area = place(source, settings.canvas, settings.fit);
            let stream = VideoStream::new(
                &entry.path,
                fps.clamp(1.0, MAX_FPS),
                (area.width, area.height),
                content_fit(settings.fit),
                true,
                control,
            )?;
            Ok(Loaded {
                area,
                content: Content::Motion(stream),
            })
        }
    }
}

struct Loader {
    entry: usize,
    handle: JoinHandle<Result<Loaded, MediaError>>,
}

/// Small deterministic generator for shuffling; playback order needs no
/// cryptographic quality.
struct Shuffler(u64);

impl Shuffler {
    fn seeded() -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_nanos() as u64)
            .unwrap_or(0x9e37_79b9_7f4a_7c15);
        Self(nanos | 1)
    }

    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    /// Fisher-Yates shuffle that never starts with `avoid`, so one item does
    /// not play twice in a row across a reshuffle.
    fn shuffle(&mut self, order: &mut [usize], avoid: Option<usize>) {
        for index in (1..order.len()).rev() {
            let pick = (self.next() % (index as u64 + 1)) as usize;
            order.swap(index, pick);
        }
        if order.len() > 1 && avoid == order.first().copied() {
            let last = order.len() - 1;
            order.swap(0, last);
        }
    }
}

struct Readout {
    label: &'static str,
    resolved: Option<ResolvedSensor>,
    retry_resolve: Option<(SensorSourceConfig, Instant)>,
    text: String,
    failed: bool,
    rendered: Option<RgbaImage>,
}

impl Readout {
    fn new(
        label: &'static str,
        category: SensorCategory,
        configured: &Option<SensorSourceConfig>,
        sensors: &[SensorInfo],
    ) -> Self {
        let source = configured
            .clone()
            .or_else(|| pick_source_for_category(category, sensors));
        let resolved = source
            .as_ref()
            .and_then(|source| resolve_sensor_source(source, sensors));
        if source.is_none() {
            warn!("playlist {label} temperature: no sensor found");
        }
        Self {
            label,
            retry_resolve: source
                .filter(|_| resolved.is_none())
                .map(|source| (source, Instant::now() + SAMPLE_INTERVAL)),
            resolved,
            text: UNAVAILABLE.into(),
            failed: false,
            rendered: None,
        }
    }

    fn sample(&mut self, now: Instant, sensors: &[SensorInfo]) {
        if let Some((source, at)) = &mut self.retry_resolve {
            if now >= *at {
                self.resolved = resolve_sensor_source(source, sensors);
                if self.resolved.is_some() {
                    debug!(
                        "playlist {} temperature sensor became available",
                        self.label
                    );
                    self.retry_resolve = None;
                } else {
                    *at = now + SAMPLE_INTERVAL;
                }
            }
        }
        let text = match self.resolved.as_ref().map(read_sensor_value) {
            Some(Ok(value)) if value.is_finite() => {
                self.failed = false;
                format!("{value:.0}°C")
            }
            Some(Ok(_)) | Some(Err(_)) => {
                if !self.failed {
                    warn!("playlist {} temperature is unavailable", self.label);
                }
                self.failed = true;
                UNAVAILABLE.into()
            }
            None => UNAVAILABLE.into(),
        };
        if text != self.text {
            self.text = text;
            self.rendered = None;
        }
    }
}

struct Overlay {
    readouts: [Option<Readout>; 2],
    sensors: Vec<SensorInfo>,
    bars: Option<[Area; 2]>,
    next_sample: Option<Instant>,
}

impl Overlay {
    fn new(config: &TemperatureOverlay, sensors: &[SensorInfo]) -> Option<Self> {
        if !config.enabled() {
            return None;
        }
        Some(Self {
            readouts: [
                config.cpu.then(|| {
                    Readout::new("CPU", SensorCategory::CpuTemp, &config.cpu_source, sensors)
                }),
                config.gpu.then(|| {
                    Readout::new("GPU", SensorCategory::GpuTemp, &config.gpu_source, sensors)
                }),
            ],
            sensors: sensors.to_vec(),
            bars: None,
            next_sample: None,
        })
    }

    fn set_bars(&mut self, bars: Option<[Area; 2]>) -> bool {
        if self.bars == bars {
            return false;
        }
        self.bars = bars;
        for readout in self.readouts.iter_mut().flatten() {
            readout.rendered = None;
        }
        true
    }

    fn update(&mut self, now: Instant, font: &FontVec) -> bool {
        let Some(bars) = self.bars else {
            self.next_sample = None;
            return false;
        };
        if self.next_sample.is_none_or(|at| now >= at) {
            self.next_sample = Some(now + SAMPLE_INTERVAL);
            for readout in self.readouts.iter_mut().flatten() {
                readout.sample(now, &self.sensors);
            }
        }
        let mut changed = false;
        for (readout, bar) in self.readouts.iter_mut().zip(bars) {
            if let Some(readout) = readout.as_mut().filter(|r| r.rendered.is_none()) {
                readout.rendered = Some(render_readout(bar, readout.label, &readout.text, font));
                changed = true;
            }
        }
        changed
    }

    fn draw(&self, canvas: &mut RgbaImage) {
        let Some(bars) = self.bars else {
            return;
        };
        for (readout, bar) in self.readouts.iter().zip(bars) {
            if let Some(image) = readout.as_ref().and_then(|r| r.rendered.as_ref()) {
                fast_overlay(canvas, image, i64::from(bar.x), i64::from(bar.y));
            }
        }
    }
}

/// Font sizes are derived from a three-digit reference so the readout keeps
/// its size as the temperature changes.
fn render_readout(bar: Area, label: &str, value: &str, font: &FontVec) -> RgbaImage {
    let mut image = RgbaImage::from_pixel(bar.width.max(1), bar.height.max(1), BAR_COLOR);
    let reference = PxScale::from(100.0);
    let (value_w, value_h, ..) = get_exact_text_metrics(font, "188°C", reference);
    let (label_w, label_h, ..) = get_exact_text_metrics(font, "GPU", reference);
    if value_w <= 0 || value_h <= 0 || label_w <= 0 || label_h <= 0 {
        return image;
    }
    const LABEL_RATIO: f32 = 0.45;
    const GAP_RATIO: f32 = 0.3;
    let padding = bar.width.min(bar.height) as f32 * 0.12;
    let box_w = (bar.width as f32 - 2.0 * padding).max(1.0);
    let box_h = (bar.height as f32 - 2.0 * padding).max(1.0);
    let by_width = box_w / (value_w as f32).max(label_w as f32 * LABEL_RATIO);
    let by_height = box_h / (value_h as f32 * (1.0 + GAP_RATIO) + label_h as f32 * LABEL_RATIO);
    let value_size = (100.0 * by_width.min(by_height)).clamp(8.0, 220.0);
    let label_size = value_size * LABEL_RATIO;

    let value_scale = PxScale::from(value_size);
    let label_scale = PxScale::from(label_size);
    let (lw, lh, lx, ly, _) = get_exact_text_metrics(font, label, label_scale);
    let (vw, vh, vx, vy, _) = get_exact_text_metrics(font, value, value_scale);
    let gap = (value_h as f32 * value_size / 100.0 * GAP_RATIO).round() as i32;
    let total = lh + gap + vh;
    let top = (bar.height as i32 - total) / 2;
    let centered = |width: i32, offset: i32| (bar.width as i32 - width) / 2 - offset;
    crate::text_raster::draw_text_mut(
        &mut image,
        LABEL_COLOR,
        centered(lw, lx),
        top - ly,
        label_scale,
        font,
        label,
    );
    crate::text_raster::draw_text_mut(
        &mut image,
        VALUE_COLOR,
        centered(vw, vx),
        top + lh + gap - vy,
        value_scale,
        font,
        value,
    );
    image
}

pub(crate) struct Playlist {
    entries: Vec<Entry>,
    order: Vec<usize>,
    cursor: usize,
    shuffle: Option<Shuffler>,
    settings: Settings,
    image_hold: Duration,
    min_motion: Duration,
    max_motion: Option<Duration>,
    active: Loaded,
    active_entry: usize,
    active_failed: bool,
    shown_at: Option<Instant>,
    loader: Option<Loader>,
    control: PreparationControl,
    failures: usize,
    reported: Vec<bool>,
    retry_at: Option<Instant>,
    overlay: Option<Overlay>,
}

impl Playlist {
    pub(crate) fn new(
        config: &PlaylistConfig,
        fit: ImageFit,
        canvas: (u32, u32),
        fps: f32,
        sensors: &[SensorInfo],
        control: &PreparationControl,
    ) -> Result<Self, MediaError> {
        config.validate().map_err(MediaError::InvalidConfig)?;
        let entries: Vec<Entry> = config
            .items
            .iter()
            .filter_map(|path| {
                PlaylistItemKind::from_path(path).map(|kind| Entry {
                    path: path.clone(),
                    kind,
                })
            })
            .collect();
        let settings = Settings {
            canvas,
            fit,
            fps: fps.clamp(1.0, MAX_FPS),
        };
        let mut shuffle = config.shuffle.then(Shuffler::seeded);
        let mut order: Vec<usize> = (0..entries.len()).collect();
        if let Some(shuffler) = &mut shuffle {
            shuffler.shuffle(&mut order, None);
        }

        let mut reported = vec![false; entries.len()];
        let mut last_error = None;
        let mut first = None;
        for (position, &entry) in order.iter().enumerate() {
            match load(&entries[entry], &settings, control) {
                Ok(loaded) => {
                    first = Some((position, entry, loaded));
                    break;
                }
                Err(MediaError::Cancelled) => return Err(MediaError::Cancelled),
                Err(error) => {
                    warn!(
                        "playlist item '{}' cannot be played: {error}",
                        entries[entry].path.display()
                    );
                    reported[entry] = true;
                    last_error = Some(error);
                }
            }
        }
        let Some((position, active_entry, active)) = first else {
            return Err(last_error.unwrap_or(MediaError::EmptyVideo));
        };

        let mut playlist = Self {
            entries,
            order,
            cursor: position + 1,
            shuffle,
            settings,
            image_hold: config.image_duration(),
            min_motion: config.min_animation_duration(),
            max_motion: config.max_animation_duration(),
            active,
            active_entry,
            active_failed: false,
            shown_at: None,
            loader: None,
            control: PreparationControl::new(control.hardware_video),
            failures: 0,
            reported,
            retry_at: None,
            overlay: Overlay::new(&config.overlay, sensors),
        };
        playlist.refresh_bars();
        Ok(playlist)
    }

    fn refresh_bars(&mut self) -> bool {
        let bars = (self.settings.fit == ImageFit::Contain)
            .then(|| side_bars(self.active.area, self.settings.canvas))
            .flatten();
        self.overlay
            .as_mut()
            .is_some_and(|overlay| overlay.set_bars(bars))
    }

    fn take_next(&mut self) -> usize {
        if self.cursor >= self.order.len() {
            self.cursor = 0;
            let avoid = Some(self.active_entry);
            if let Some(shuffler) = &mut self.shuffle {
                shuffler.shuffle(&mut self.order, avoid);
            }
        }
        let entry = self.order[self.cursor];
        self.cursor += 1;
        entry
    }

    fn start_loading(&mut self, now: Instant) {
        if self.loader.is_some() || self.retry_at.is_some_and(|at| now < at) {
            return;
        }
        if self.entries.len() == 1 && !self.active_failed {
            return;
        }
        self.retry_at = None;
        let entry = self.take_next();
        let item = Entry {
            path: self.entries[entry].path.clone(),
            kind: self.entries[entry].kind,
        };
        let settings = self.settings;
        let control = self.control.next_asset();
        let spawned = thread::Builder::new()
            .name("playlist-load".into())
            .spawn(move || load(&item, &settings, &control));
        match spawned {
            Ok(handle) => self.loader = Some(Loader { entry, handle }),
            Err(error) => {
                warn!("playlist could not start loading the next item: {error}");
                self.retry_at = Some(now + FAILURE_BACKOFF);
            }
        }
    }

    fn active_done(&self, now: Instant) -> bool {
        if self.active_failed {
            return true;
        }
        if self.entries.len() == 1 {
            return false;
        }
        let Some(shown_at) = self.shown_at else {
            return false;
        };
        let elapsed = now.saturating_duration_since(shown_at);
        match &self.active.content {
            Content::Still { .. } => elapsed >= self.image_hold,
            Content::Motion(stream) => {
                self.max_motion.is_some_and(|max| elapsed >= max)
                    || (elapsed >= self.min_motion && stream.first_pass_complete(now))
            }
        }
    }

    /// Swaps in the preloaded item once it is ready. Until then the current
    /// item keeps playing, so a slow load never blanks the screen.
    fn try_switch(&mut self, now: Instant) -> bool {
        if !self
            .loader
            .as_ref()
            .is_some_and(|loader| loader.handle.is_finished())
        {
            self.start_loading(now);
            return false;
        }
        let loader = self.loader.take().expect("finished loader");
        let result = loader
            .handle
            .join()
            .unwrap_or_else(|_| Err(MediaError::Ffmpeg("Playlist loader panicked".into())));
        match result {
            Ok(mut loaded) => {
                let started = match &mut loaded.content {
                    // Starts the clip's clock on the frame that first shows it.
                    Content::Motion(stream) => stream.advance(now).map(|_| ()),
                    Content::Still { .. } => Ok(()),
                };
                self.reported[loader.entry] = false;
                self.active = loaded;
                self.active_entry = loader.entry;
                self.active_failed = started.is_err();
                if let Err(error) = started {
                    self.report_failure(loader.entry, &error);
                }
                self.shown_at = Some(now);
                self.failures = 0;
                self.refresh_bars();
                self.start_loading(now);
                true
            }
            Err(error) => {
                self.report_failure(loader.entry, &error);
                self.failures += 1;
                if self.failures >= self.entries.len() {
                    self.failures = 0;
                    self.retry_at = Some(now + FAILURE_BACKOFF);
                }
                self.start_loading(now);
                false
            }
        }
    }

    /// Warns once per item until it plays again, so a broken file in a
    /// looping playlist does not repeat the same warning every cycle.
    fn report_failure(&mut self, entry: usize, error: &MediaError) {
        if matches!(error, MediaError::Cancelled)
            || std::mem::replace(&mut self.reported[entry], true)
        {
            return;
        }
        warn!(
            "playlist item '{}' cannot be played: {error}",
            self.entries[entry].path.display()
        );
    }

    pub(crate) fn advance(&mut self, now: Instant, font: &FontVec) -> bool {
        let mut changed = false;
        if self.shown_at.is_none() {
            self.shown_at = Some(now);
            changed = true;
        }
        let playback = match &mut self.active.content {
            Content::Motion(stream) if !self.active_failed => stream.advance(now),
            _ => Ok(false),
        };
        match playback {
            Ok(advanced) => changed |= advanced,
            Err(error) => {
                self.active_failed = true;
                self.report_failure(self.active_entry, &error);
            }
        }
        if self.active_done(now) {
            changed |= self.try_switch(now);
        } else {
            self.start_loading(now);
        }
        if let Some(overlay) = &mut self.overlay {
            changed |= overlay.update(now, font);
        }
        changed
    }

    pub(crate) fn draw(&self, canvas: &mut RgbaImage) {
        let area = self.active.area;
        let pixels = match &self.active.content {
            Content::Still { pixels, .. } => pixels,
            Content::Motion(stream) => stream.frame(),
        };
        fast_overlay(canvas, pixels, i64::from(area.x), i64::from(area.y));
        if let Some(overlay) = &self.overlay {
            overlay.draw(canvas);
        }
    }
}

impl Drop for Playlist {
    fn drop(&mut self) {
        // A pending load observes the cancellation, releases its decoder and
        // exits on its own; joining here would stall the renderer.
        self.control.cancel();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PORTRAIT: (u32, u32) = (720, 1472);
    const LANDSCAPE: (u32, u32) = (1472, 720);

    #[test]
    fn contain_keeps_aspect_ratio_and_centres_media() {
        assert_eq!(
            place((1920, 1080), PORTRAIT, ImageFit::Contain),
            Area {
                x: 0,
                y: 533,
                width: 720,
                height: 405
            }
        );
        assert_eq!(
            place((1920, 1080), LANDSCAPE, ImageFit::Contain),
            Area {
                x: 96,
                y: 0,
                width: 1280,
                height: 720
            }
        );
        assert_eq!(
            place((1920, 1080), PORTRAIT, ImageFit::Cover),
            Area {
                x: 0,
                y: 0,
                width: 720,
                height: 1472
            }
        );
    }

    #[test]
    fn readouts_use_whichever_bars_fit_mode_leaves() {
        let beside = side_bars(place((1920, 1080), LANDSCAPE, ImageFit::Contain), LANDSCAPE)
            .expect("16:9 on the landscape panel leaves 96px side bars");
        assert_eq!(
            beside[0],
            Area {
                x: 0,
                y: 0,
                width: 96,
                height: 720
            }
        );
        assert_eq!(
            beside[1],
            Area {
                x: 1376,
                y: 0,
                width: 96,
                height: 720
            }
        );

        let stacked = side_bars(place((1920, 1080), PORTRAIT, ImageFit::Contain), PORTRAIT)
            .expect("16:9 on the portrait panel leaves top and bottom bars");
        assert_eq!(
            stacked[0],
            Area {
                x: 0,
                y: 0,
                width: 720,
                height: 533
            }
        );
        assert_eq!(
            stacked[1],
            Area {
                x: 0,
                y: 938,
                width: 720,
                height: 534
            }
        );
    }

    #[test]
    fn readouts_hide_when_media_leaves_no_usable_bar() {
        let full = place((720, 1472), PORTRAIT, ImageFit::Contain);
        assert_eq!(side_bars(full, PORTRAIT), None);
        let thin = place((700, 1472), PORTRAIT, ImageFit::Contain);
        assert!(thin.x > 0 && thin.x < MIN_BAR);
        assert_eq!(side_bars(thin, PORTRAIT), None);
    }

    #[test]
    fn reshuffle_never_repeats_the_item_that_just_played() {
        let mut shuffler = Shuffler(12345);
        for round in 0..200 {
            let mut order = vec![0, 1, 2, 3];
            let avoid = round % 4;
            shuffler.shuffle(&mut order, Some(avoid));
            assert_ne!(order[0], avoid);
            let mut sorted = order.clone();
            sorted.sort_unstable();
            assert_eq!(sorted, vec![0, 1, 2, 3]);
        }
    }

    #[test]
    fn readout_text_stays_inside_its_bar() {
        let font = crate::fonts::load(
            &std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../templates/assets/neon-us88/JetBrainsMonoNL-Medium.ttf"),
        )
        .unwrap();
        for bar in [
            Area {
                x: 0,
                y: 0,
                width: 96,
                height: 720,
            },
            Area {
                x: 0,
                y: 0,
                width: 720,
                height: 533,
            },
            Area {
                x: 0,
                y: 0,
                width: 64,
                height: 64,
            },
        ] {
            let image = render_readout(bar, "GPU", "100°C", &font);
            let lit: Vec<(u32, u32)> = image
                .enumerate_pixels()
                .filter(|(_, _, pixel)| pixel.0[0] > 0)
                .map(|(x, y, _)| (x, y))
                .collect();
            assert!(!lit.is_empty(), "{bar:?} rendered no text");
            let (min_x, max_x) = lit
                .iter()
                .fold((u32::MAX, 0), |(lo, hi), &(x, _)| (lo.min(x), hi.max(x)));
            let (min_y, max_y) = lit
                .iter()
                .fold((u32::MAX, 0), |(lo, hi), &(_, y)| (lo.min(y), hi.max(y)));
            assert!(
                min_x > 0 && max_x < bar.width - 1,
                "{bar:?} clipped horizontally"
            );
            assert!(
                min_y > 0 && max_y < bar.height - 1,
                "{bar:?} clipped vertically"
            );
        }
    }

    fn test_font() -> FontVec {
        crate::fonts::load(
            &std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../templates/assets/neon-us88/JetBrainsMonoNL-Medium.ttf"),
        )
        .unwrap()
    }

    fn still(path: &std::path::Path, rgb: [u8; 3]) -> PathBuf {
        image::RgbImage::from_pixel(32, 16, image::Rgb(rgb))
            .save(path)
            .unwrap();
        path.to_owned()
    }

    fn two_frame_gif(path: &std::path::Path) -> PathBuf {
        let file = std::fs::File::create(path).unwrap();
        let mut encoder = image::codecs::gif::GifEncoder::new(file);
        for rgb in [[0, 255, 0, 255], [0, 128, 0, 255]] {
            let frame = image::Frame::from_parts(
                RgbaImage::from_pixel(32, 16, Rgba(rgb)),
                0,
                0,
                image::Delay::from_numer_denom_ms(100, 1),
            );
            encoder.encode_frame(frame).unwrap();
        }
        drop(encoder);
        path.to_owned()
    }

    fn wait_for_loader(playlist: &Playlist) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !playlist
            .loader
            .as_ref()
            .is_some_and(|loader| loader.handle.is_finished())
        {
            assert!(Instant::now() < deadline, "next item never loaded");
            thread::sleep(Duration::from_millis(5));
        }
    }

    fn centre(playlist: &Playlist, canvas: (u32, u32)) -> [u8; 4] {
        let mut image = RgbaImage::from_pixel(canvas.0, canvas.1, BAR_COLOR);
        playlist.draw(&mut image);
        image.get_pixel(canvas.0 / 2, canvas.1 / 2).0
    }

    #[test]
    fn mixed_playlist_holds_stills_plays_animations_once_and_loops() {
        let root = tempfile::tempdir().unwrap();
        let config = PlaylistConfig {
            items: vec![
                still(&root.path().join("red.png"), [255, 0, 0]),
                two_frame_gif(&root.path().join("green.gif")),
                still(&root.path().join("blue.png"), [0, 0, 255]),
            ],
            image_seconds: 1,
            ..Default::default()
        };
        let canvas = (64, 64);
        let font = test_font();
        let mut playlist = Playlist::new(
            &config,
            ImageFit::Contain,
            canvas,
            10.0,
            &[],
            &PreparationControl::new(false),
        )
        .unwrap();
        let start = Instant::now();
        assert!(playlist.advance(start, &font));
        assert_eq!(centre(&playlist, canvas), [255, 0, 0, 255]);

        wait_for_loader(&playlist);
        assert!(!playlist.advance(start + Duration::from_millis(999), &font));
        assert_eq!(playlist.active_entry, 0, "image left before its hold time");
        assert!(playlist.advance(start + Duration::from_secs(1), &font));
        assert_eq!(playlist.active_entry, 1);
        assert_eq!(centre(&playlist, canvas)[1], 255);

        let gif_start = start + Duration::from_secs(1);
        wait_for_loader(&playlist);
        playlist.advance(gif_start + Duration::from_millis(100), &font);
        assert_eq!(playlist.active_entry, 1, "GIF ended before its last frame");
        assert!(playlist.advance(gif_start + Duration::from_millis(200), &font));
        assert_eq!(playlist.active_entry, 2);
        assert_eq!(centre(&playlist, canvas), [0, 0, 255, 255]);

        wait_for_loader(&playlist);
        assert!(playlist.advance(gif_start + Duration::from_millis(1200), &font));
        assert_eq!(playlist.active_entry, 0, "playlist did not loop");
    }

    #[test]
    fn minimum_and_maximum_durations_bound_animation_playback() {
        let root = tempfile::tempdir().unwrap();
        let gif = two_frame_gif(&root.path().join("short.gif"));
        let config = PlaylistConfig {
            items: vec![gif, still(&root.path().join("after.png"), [9, 9, 9])],
            min_animation_seconds: 1,
            ..Default::default()
        };
        let font = test_font();
        let mut playlist = Playlist::new(
            &config,
            ImageFit::Contain,
            (64, 64),
            10.0,
            &[],
            &PreparationControl::new(false),
        )
        .unwrap();
        let start = Instant::now();
        playlist.advance(start, &font);
        wait_for_loader(&playlist);
        playlist.advance(start + Duration::from_millis(500), &font);
        assert_eq!(
            playlist.active_entry, 0,
            "short GIF did not loop to the minimum"
        );
        playlist.advance(start + Duration::from_secs(1), &font);
        assert_eq!(playlist.active_entry, 1);

        let capped = PlaylistConfig {
            max_animation_seconds: 1,
            min_animation_seconds: 0,
            ..config
        };
        let mut playlist = Playlist::new(
            &capped,
            ImageFit::Contain,
            (64, 64),
            10.0,
            &[],
            &PreparationControl::new(false),
        )
        .unwrap();
        playlist.max_motion = Some(Duration::from_millis(150));
        playlist.advance(start, &font);
        wait_for_loader(&playlist);
        playlist.advance(start + Duration::from_millis(150), &font);
        assert_eq!(
            playlist.active_entry, 1,
            "maximum did not cut the GIF short"
        );
    }

    #[test]
    fn single_item_keeps_playing_without_reloading() {
        let root = tempfile::tempdir().unwrap();
        let config = PlaylistConfig {
            items: vec![still(&root.path().join("only.png"), [1, 2, 3])],
            image_seconds: 1,
            ..Default::default()
        };
        let font = test_font();
        let mut playlist = Playlist::new(
            &config,
            ImageFit::Contain,
            (64, 64),
            10.0,
            &[],
            &PreparationControl::new(false),
        )
        .unwrap();
        let start = Instant::now();
        assert!(playlist.advance(start, &font));
        assert!(!playlist.advance(start + Duration::from_secs(30), &font));
        assert!(playlist.loader.is_none());
    }

    #[test]
    fn unplayable_items_are_skipped_and_the_rest_still_play() {
        let root = tempfile::tempdir().unwrap();
        let broken = root.path().join("broken.png");
        std::fs::write(&broken, b"not an image").unwrap();
        let config = PlaylistConfig {
            items: vec![
                broken.clone(),
                still(&root.path().join("ok.png"), [7, 7, 7]),
            ],
            image_seconds: 1,
            ..Default::default()
        };
        let font = test_font();
        let mut playlist = Playlist::new(
            &config,
            ImageFit::Contain,
            (64, 64),
            10.0,
            &[],
            &PreparationControl::new(false),
        )
        .unwrap();
        assert_eq!(playlist.active_entry, 1);
        let start = Instant::now();
        playlist.advance(start, &font);
        wait_for_loader(&playlist);
        assert!(!playlist.advance(start + Duration::from_secs(1), &font));
        assert_eq!(
            playlist.active_entry, 1,
            "a failed load replaced the screen"
        );

        let only_broken = PlaylistConfig {
            items: vec![broken],
            ..Default::default()
        };
        assert!(Playlist::new(
            &only_broken,
            ImageFit::Contain,
            (64, 64),
            10.0,
            &[],
            &PreparationControl::new(false),
        )
        .is_err());
    }

    #[test]
    fn overlay_draws_constant_readings_only_inside_the_bars() {
        let root = tempfile::tempdir().unwrap();
        let config = PlaylistConfig {
            items: vec![still(&root.path().join("wide.png"), [200, 0, 0])],
            overlay: TemperatureOverlay {
                cpu: true,
                gpu: true,
                cpu_source: Some(SensorSourceConfig::Constant { value: 47.0 }),
                gpu_source: Some(SensorSourceConfig::Command {
                    cmd: "exit 1".into(),
                }),
            },
            ..Default::default()
        };
        let canvas = (128, 256);
        let font = test_font();
        let mut playlist = Playlist::new(
            &config,
            ImageFit::Contain,
            canvas,
            10.0,
            &[],
            &PreparationControl::new(false),
        )
        .unwrap();
        playlist.advance(Instant::now(), &font);
        let overlay = playlist.overlay.as_ref().unwrap();
        let readouts: Vec<&str> = overlay
            .readouts
            .iter()
            .flatten()
            .map(|readout| readout.text.as_str())
            .collect();
        assert_eq!(readouts, ["47°C", UNAVAILABLE]);

        let mut image = RgbaImage::from_pixel(canvas.0, canvas.1, BAR_COLOR);
        playlist.draw(&mut image);
        let media = playlist.active.area;
        assert_eq!(
            media,
            Area {
                x: 0,
                y: 96,
                width: 128,
                height: 64
            }
        );
        let text_rows = |rows: std::ops::Range<u32>| {
            rows.flat_map(|y| (0..canvas.0).map(move |x| (x, y)))
                .filter(|&(x, y)| image.get_pixel(x, y).0[1] > 0)
                .count()
        };
        assert!(text_rows(0..96) > 0, "CPU readout missing from the top bar");
        assert!(
            text_rows(160..256) > 0,
            "GPU readout missing from the bottom bar"
        );
        assert!(
            (96..160).all(|y| (0..canvas.0).all(|x| image.get_pixel(x, y).0 == [200, 0, 0, 255])),
            "overlay covered the media"
        );
    }

    #[test]
    fn overlay_hides_when_fill_modes_leave_no_bars() {
        let root = tempfile::tempdir().unwrap();
        let config = PlaylistConfig {
            items: vec![still(&root.path().join("wide.png"), [200, 0, 0])],
            overlay: TemperatureOverlay {
                cpu: true,
                cpu_source: Some(SensorSourceConfig::Constant { value: 47.0 }),
                ..Default::default()
            },
            ..Default::default()
        };
        let font = test_font();
        let mut playlist = Playlist::new(
            &config,
            ImageFit::Cover,
            (128, 256),
            10.0,
            &[],
            &PreparationControl::new(false),
        )
        .unwrap();
        playlist.advance(Instant::now(), &font);
        assert_eq!(playlist.overlay.as_ref().unwrap().bars, None);
    }

    #[test]
    fn rotated_video_is_placed_by_its_display_size() {
        let root = tempfile::tempdir().unwrap();
        let plain = root.path().join("plain.mp4");
        let rotated = root.path().join("rotated.mp4");
        let run = |args: &[&str]| {
            let output = std::process::Command::new("ffmpeg")
                .args(["-v", "error", "-y"])
                .args(args)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
        };
        run(&[
            "-f",
            "lavfi",
            "-i",
            "testsrc=size=32x16:rate=10",
            "-t",
            "0.5",
            "-pix_fmt",
            "yuv420p",
            plain.to_str().unwrap(),
        ]);
        run(&[
            "-display_rotation",
            "90",
            "-i",
            plain.to_str().unwrap(),
            "-c",
            "copy",
            rotated.to_str().unwrap(),
        ]);
        let config = PlaylistConfig {
            items: vec![rotated],
            ..Default::default()
        };
        let font = test_font();
        let mut playlist = Playlist::new(
            &config,
            ImageFit::Contain,
            (64, 64),
            10.0,
            &[],
            &PreparationControl::new(false),
        )
        .unwrap();
        assert_eq!(
            playlist.active.area,
            Area {
                x: 16,
                y: 0,
                width: 32,
                height: 64
            }
        );
        assert!(playlist.advance(Instant::now(), &font));
        let Content::Motion(stream) = &playlist.active.content else {
            panic!("video loaded as a still");
        };
        assert_eq!(stream.frame().dimensions(), (32, 64));
    }
}
