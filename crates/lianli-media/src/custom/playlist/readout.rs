//! Readouts drawn into the bars beside playlist media.

use super::layout::Area;
use crate::common::get_exact_text_metrics;
use crate::custom::helpers::{fast_overlay, resolve_sensor_source};
use ab_glyph::{FontVec, PxScale};
use image::{Rgba, RgbaImage};
use imageproc::drawing::draw_antialiased_line_segment_mut;
use imageproc::pixelops::interpolate;
use lianli_shared::media::SensorSourceConfig;
use lianli_shared::playlist::{ReadoutKind, ReadoutPreview, ReadoutSlot, TemperatureOverlay};
use lianli_shared::sensors::{
    find_default_gpu_hotspot, pick_source_for_category, read_sensor_value, ResolvedSensor,
    SensorCategory, SensorInfo, SensorSource,
};
use std::collections::VecDeque;
use std::time::{Duration, Instant};
use tracing::{debug, warn};

pub(super) const SAMPLE_INTERVAL: Duration = Duration::from_secs(2);
const HISTORY: usize = 30;
pub(super) const UNAVAILABLE: &str = "--";
const LABEL_COLOR: Rgba<u8> = Rgba([170, 170, 170, 255]);
const VALUE_COLOR: Rgba<u8> = Rgba([255, 255, 255, 255]);
const WARM_COLOR: Rgba<u8> = Rgba([245, 165, 40, 255]);
const HOT_COLOR: Rgba<u8> = Rgba([235, 60, 50, 255]);
const SPARK_COLOR: Rgba<u8> = Rgba([120, 180, 255, 255]);
pub(super) const BAR_COLOR: Rgba<u8> = Rgba([0, 0, 0, 255]);

#[derive(Clone, Copy)]
struct Style {
    fahrenheit: bool,
    colors: bool,
    warm_c: f32,
    hot_c: f32,
}

fn sensor_source(
    kind: ReadoutKind,
    slot: &ReadoutSlot,
    sensors: &[SensorInfo],
) -> Option<SensorSourceConfig> {
    if let Some(source) = &slot.source {
        return Some(source.clone());
    }
    match kind {
        ReadoutKind::CpuTemp => pick_source_for_category(SensorCategory::CpuTemp, sensors),
        ReadoutKind::GpuTemp => pick_source_for_category(SensorCategory::GpuTemp, sensors),
        ReadoutKind::CpuUsage => Some(SensorSourceConfig::CpuUsage),
        ReadoutKind::GpuUsage => pick_source_for_category(SensorCategory::GpuUsage, sensors),
        ReadoutKind::GpuHotspot => match find_default_gpu_hotspot(sensors)? {
            SensorSource::Hwmon {
                name,
                label,
                device_path,
            } => Some(SensorSourceConfig::Hwmon {
                name,
                label,
                device_path,
            }),
            _ => None,
        },
        ReadoutKind::Clock | ReadoutKind::Sensor | ReadoutKind::Off => None,
    }
}

fn default_label(kind: ReadoutKind) -> &'static str {
    match kind {
        ReadoutKind::CpuTemp | ReadoutKind::CpuUsage => "CPU",
        ReadoutKind::GpuTemp | ReadoutKind::GpuUsage => "GPU",
        ReadoutKind::GpuHotspot => "HOTSPOT",
        ReadoutKind::Clock => "TIME",
        ReadoutKind::Sensor | ReadoutKind::Off => "",
    }
}

pub(super) struct Readout {
    kind: ReadoutKind,
    label: String,
    unit: String,
    sensor_name: Option<String>,
    resolved: Option<ResolvedSensor>,
    retry_resolve: Option<(SensorSourceConfig, Instant)>,
    pub(super) text: String,
    color: Rgba<u8>,
    sparkline: bool,
    history: VecDeque<f32>,
    failed: bool,
    rendered: Option<RgbaImage>,
}

impl Readout {
    fn new(slot: &ReadoutSlot, sensors: &[SensorInfo]) -> Option<Self> {
        Self::build(slot, sensors, true)
    }

    /// `report` is false for previews, which are rebuilt every few seconds
    /// and would otherwise repeat the same warnings.
    fn build(slot: &ReadoutSlot, sensors: &[SensorInfo], report: bool) -> Option<Self> {
        let kind = slot.kind;
        if kind == ReadoutKind::Off {
            return None;
        }
        let source = sensor_source(kind, slot, sensors);
        let target = source.as_ref().map(SensorSourceConfig::to_sensor_source);
        let info = target
            .as_ref()
            .and_then(|target| sensors.iter().find(|sensor| &sensor.source == target));
        let resolved = source
            .as_ref()
            .and_then(|source| resolve_sensor_source(source, sensors));
        let label = slot
            .label
            .clone()
            .filter(|label| !label.trim().is_empty())
            .unwrap_or_else(|| default_label(kind).to_string());
        if report && source.is_none() && kind != ReadoutKind::Clock {
            warn!("playlist {label} readout: no sensor found");
        }
        Some(Self {
            kind,
            label,
            unit: info
                .map(|sensor| sensor.unit.to_string())
                .unwrap_or_default(),
            sensor_name: info.map(SensorInfo::get_display_name),
            retry_resolve: source
                .filter(|_| resolved.is_none())
                .map(|source| (source, Instant::now() + SAMPLE_INTERVAL)),
            resolved,
            text: UNAVAILABLE.into(),
            color: VALUE_COLOR,
            sparkline: slot.sparkline && kind != ReadoutKind::Clock,
            history: VecDeque::new(),
            failed: !report,
            rendered: None,
        })
    }

    fn format(&self, value: f32, style: Style) -> String {
        match self.kind {
            kind if kind.is_temperature() => {
                if style.fahrenheit {
                    format!("{:.0}°F", value * 9.0 / 5.0 + 32.0)
                } else {
                    format!("{value:.0}°C")
                }
            }
            ReadoutKind::CpuUsage | ReadoutKind::GpuUsage => format!("{value:.0}%"),
            _ if self.unit.is_empty() => format!("{value:.0}"),
            _ if self.unit.starts_with(['°', '%']) => format!("{value:.0}{}", self.unit),
            _ => format!("{value:.0} {}", self.unit),
        }
    }

    /// Widest text the readout is expected to show, so its size stays put
    /// as the value changes.
    fn reference(&self, style: Style) -> String {
        match self.kind {
            ReadoutKind::Clock => "88:88".into(),
            kind if kind.is_temperature() && style.fahrenheit => "388°F".into(),
            kind if kind.is_temperature() => "188°C".into(),
            ReadoutKind::CpuUsage | ReadoutKind::GpuUsage => "100%".into(),
            _ => self.format(8888.0, style),
        }
    }

    fn color_for(&self, value: f32, style: Style) -> Rgba<u8> {
        if !style.colors || !self.kind.is_temperature() {
            VALUE_COLOR
        } else if value >= style.hot_c {
            HOT_COLOR
        } else if value >= style.warm_c {
            WARM_COLOR
        } else {
            VALUE_COLOR
        }
    }

    fn sample(&mut self, now: Instant, sensors: &[SensorInfo], style: Style) {
        if self.kind == ReadoutKind::Clock {
            let text = chrono::Local::now().format("%H:%M").to_string();
            if text != self.text {
                self.text = text;
                self.rendered = None;
            }
            return;
        }
        if let Some((source, at)) = &mut self.retry_resolve {
            if now >= *at {
                self.resolved = resolve_sensor_source(source, sensors);
                if self.resolved.is_some() {
                    debug!("playlist {} readout sensor became available", self.label);
                    self.retry_resolve = None;
                } else {
                    *at = now + SAMPLE_INTERVAL;
                }
            }
        }
        let value = match self.resolved.as_ref().map(read_sensor_value) {
            Some(Ok(value)) if value.is_finite() => {
                self.failed = false;
                Some(value)
            }
            Some(_) => {
                if !self.failed {
                    warn!("playlist {} readout is unavailable", self.label);
                }
                self.failed = true;
                None
            }
            None => None,
        };
        let (text, color) = match value {
            Some(value) => (self.format(value, style), self.color_for(value, style)),
            None => (UNAVAILABLE.to_string(), VALUE_COLOR),
        };
        if self.sparkline {
            if let Some(value) = value {
                if self.history.len() == HISTORY {
                    self.history.pop_front();
                }
                self.history.push_back(value);
                self.rendered = None;
            }
        }
        if text != self.text || color != self.color {
            self.text = text;
            self.color = color;
            self.rendered = None;
        }
    }
}

pub(super) struct Overlay {
    pub(super) readouts: [Option<Readout>; 2],
    sensors: Vec<SensorInfo>,
    pub(super) bars: Option<[Area; 2]>,
    next_sample: Option<Instant>,
    font: Option<FontVec>,
    style: Style,
    /// Changes whenever the drawn readouts change, for frame caching.
    pub(super) version: u64,
    max_font: f32,
}

impl Overlay {
    pub(super) fn new(config: &TemperatureOverlay, sensors: &[SensorInfo]) -> Option<Self> {
        if !config.enabled() {
            return None;
        }
        let [first, second] = config.effective_slots();
        let font = config.font_path.as_ref().and_then(|path| {
            crate::fonts::load(path)
                .map_err(|error| {
                    warn!(
                        "playlist readout font '{}' cannot be used: {error}",
                        path.display()
                    )
                })
                .ok()
        });
        Some(Self {
            readouts: [
                Readout::new(&first, sensors),
                Readout::new(&second, sensors),
            ],
            sensors: sensors.to_vec(),
            bars: None,
            next_sample: None,
            font,
            style: Style {
                fahrenheit: config.fahrenheit,
                colors: config.colors,
                warm_c: config.warm_c,
                hot_c: config.hot_c,
            },
            version: 0,
            max_font: 220.0,
        })
    }

    pub(super) fn set_bars(&mut self, bars: Option<[Area; 2]>, max_font: f32) -> bool {
        if self.bars == bars && self.max_font == max_font {
            return false;
        }
        self.bars = bars;
        self.max_font = max_font;
        for readout in self.readouts.iter_mut().flatten() {
            readout.rendered = None;
        }
        self.version += 1;
        true
    }

    pub(super) fn update(&mut self, now: Instant, default_font: &FontVec) -> bool {
        let Some(bars) = self.bars else {
            self.next_sample = None;
            return false;
        };
        if self.next_sample.is_none_or(|at| now >= at) {
            self.next_sample = Some(now + SAMPLE_INTERVAL);
            for readout in self.readouts.iter_mut().flatten() {
                readout.sample(now, &self.sensors, self.style);
            }
        }
        let font = self.font.as_ref().unwrap_or(default_font);
        let mut changed = false;
        for (readout, bar) in self.readouts.iter_mut().zip(bars) {
            if let Some(readout) = readout.as_mut().filter(|r| r.rendered.is_none()) {
                readout.rendered = Some(render_readout(
                    bar,
                    readout,
                    &readout.reference(self.style),
                    font,
                    self.max_font,
                ));
                changed = true;
            }
        }
        if changed {
            self.version += 1;
        }
        changed
    }

    pub(super) fn draw(&self, canvas: &mut RgbaImage) {
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

/// Current readouts for `config`, sampled once, for the settings preview.
/// Command sources are not run: previews are requested without the write
/// guard, so they must not execute anything a client supplies.
pub(crate) fn preview(config: &TemperatureOverlay, sensors: &[SensorInfo]) -> Vec<ReadoutPreview> {
    let style = Style {
        fahrenheit: config.fahrenheit,
        colors: false,
        warm_c: config.warm_c,
        hot_c: config.hot_c,
    };
    config
        .effective_slots()
        .iter()
        .map(|slot| {
            Readout::build(slot, sensors, false).map_or_else(
                ReadoutPreview::default,
                |mut readout| {
                    if matches!(slot.source, Some(SensorSourceConfig::Command { .. })) {
                        return ReadoutPreview {
                            label: readout.label,
                            text: UNAVAILABLE.into(),
                            sensor: None,
                        };
                    }
                    readout.sample(Instant::now(), sensors, style);
                    ReadoutPreview {
                        label: readout.label,
                        text: readout.text,
                        sensor: readout.sensor_name,
                    }
                },
            )
        })
        .collect()
}

fn draw_sparkline(image: &mut RgbaImage, history: &VecDeque<f32>, area: Area) {
    if history.len() < 2 || area.width < 4 || area.height < 4 {
        return;
    }
    let (min, max) = history
        .iter()
        .fold((f32::MAX, f32::MIN), |(lo, hi), &v| (lo.min(v), hi.max(v)));
    let span = (max - min).max(1.0);
    let step = (area.width - 1) as f32 / (HISTORY - 1) as f32;
    let start = HISTORY - history.len();
    let point = |index: usize, value: f32| {
        let x = area.x as f32 + (start + index) as f32 * step;
        let y = area.y as f32 + (area.height - 1) as f32 * (1.0 - (value - min) / span);
        (x.round() as i32, y.round() as i32)
    };
    for (index, pair) in history.iter().collect::<Vec<_>>().windows(2).enumerate() {
        draw_antialiased_line_segment_mut(
            image,
            point(index, *pair[0]),
            point(index + 1, *pair[1]),
            SPARK_COLOR,
            interpolate,
        );
    }
}

/// Lays out the label, value and optional sparkline as a centred column.
pub(super) fn render_readout(
    bar: Area,
    readout: &Readout,
    reference: &str,
    font: &FontVec,
    max_font: f32,
) -> RgbaImage {
    let mut image = RgbaImage::from_pixel(bar.width.max(1), bar.height.max(1), BAR_COLOR);
    let unit = PxScale::from(100.0);
    let (value_w, value_h, ..) = get_exact_text_metrics(font, reference, unit);
    let label_ref = if readout.label.is_empty() {
        "GPU"
    } else {
        &readout.label
    };
    let (label_w, label_h, ..) = get_exact_text_metrics(font, label_ref, unit);
    if value_w <= 0 || value_h <= 0 {
        return image;
    }
    const LABEL_RATIO: f32 = 0.45;
    const GAP_RATIO: f32 = 0.3;
    const SPARK_RATIO: f32 = 0.7;
    let label_h = if readout.label.is_empty() {
        0
    } else {
        label_h.max(1)
    };
    let spark = if readout.sparkline { SPARK_RATIO } else { 0.0 };
    let padding = bar.width.min(bar.height) as f32 * 0.12;
    let box_w = (bar.width as f32 - 2.0 * padding).max(1.0);
    let box_h = (bar.height as f32 - 2.0 * padding).max(1.0);
    let by_width = box_w / (value_w as f32).max(label_w as f32 * LABEL_RATIO);
    let rows = value_h as f32 * (1.0 + spark + if label_h > 0 { GAP_RATIO } else { 0.0 })
        + if readout.sparkline {
            value_h as f32 * GAP_RATIO
        } else {
            0.0
        }
        + label_h as f32 * LABEL_RATIO;
    let value_size = (100.0 * by_width.min(box_h / rows)).clamp(8.0, max_font);
    let scale = value_size / 100.0;

    let value_scale = PxScale::from(value_size);
    let label_scale = PxScale::from(value_size * LABEL_RATIO);
    let (lw, lh, lx, ly, _) = if readout.label.is_empty() {
        (0, 0, 0, 0, 0.0)
    } else {
        get_exact_text_metrics(font, &readout.label, label_scale)
    };
    let (vw, vh, vx, vy, _) = get_exact_text_metrics(font, &readout.text, value_scale);
    let gap = (value_h as f32 * scale * GAP_RATIO).round() as i32;
    let spark_h = (value_h as f32 * scale * spark).round() as i32;
    let total =
        lh + if lh > 0 { gap } else { 0 } + vh + if readout.sparkline { gap + spark_h } else { 0 };
    let top = (bar.height as i32 - total) / 2;
    let centered = |width: i32, offset: i32| (bar.width as i32 - width) / 2 - offset;
    if lh > 0 {
        crate::text_raster::draw_text_mut(
            &mut image,
            LABEL_COLOR,
            centered(lw, lx),
            top - ly,
            label_scale,
            font,
            &readout.label,
        );
    }
    let value_top = top + if lh > 0 { lh + gap } else { 0 };
    crate::text_raster::draw_text_mut(
        &mut image,
        readout.color,
        centered(vw, vx),
        value_top - vy,
        value_scale,
        font,
        &readout.text,
    );
    if readout.sparkline {
        let width = (box_w * 0.85).round().max(4.0) as u32;
        draw_sparkline(
            &mut image,
            &readout.history,
            Area {
                x: (bar.width.saturating_sub(width)) / 2,
                y: (value_top + vh + gap).max(0) as u32,
                width,
                height: spark_h.max(4) as u32,
            },
        );
    }
    image
}

#[cfg(test)]
mod tests {
    use super::*;
    use lianli_shared::playlist::ReadoutSlot;

    fn font() -> FontVec {
        crate::fonts::load(
            &std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../templates/assets/neon-us88/JetBrainsMonoNL-Medium.ttf"),
        )
        .unwrap()
    }

    fn constant(kind: ReadoutKind, value: f32) -> ReadoutSlot {
        ReadoutSlot {
            kind,
            source: Some(SensorSourceConfig::Constant { value }),
            ..Default::default()
        }
    }

    fn style() -> Style {
        Style {
            fahrenheit: false,
            colors: true,
            warm_c: 75.0,
            hot_c: 85.0,
        }
    }

    fn lit_bounds(image: &RgbaImage) -> Option<(u32, u32, u32, u32)> {
        image
            .enumerate_pixels()
            .filter(|(_, _, pixel)| pixel.0[..3].iter().any(|&c| c > 0))
            .fold(None, |bounds, (x, y, _)| {
                let (x0, y0, x1, y1) = bounds.unwrap_or((x, y, x, y));
                Some((x0.min(x), y0.min(y), x1.max(x), y1.max(y)))
            })
    }

    #[test]
    fn readout_text_and_sparkline_stay_inside_their_bar() {
        let font = font();
        let mut readout = Readout::new(
            &ReadoutSlot {
                sparkline: true,
                ..constant(ReadoutKind::GpuTemp, 61.0)
            },
            &[],
        )
        .unwrap();
        for _ in 0..5 {
            readout.sample(Instant::now(), &[], style());
        }
        readout.history.push_back(70.0);
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
            let image = render_readout(bar, &readout, "188°C", &font, 220.0);
            let (x0, y0, x1, y1) = lit_bounds(&image).expect("rendered nothing");
            assert!(x0 > 0 && x1 < bar.width - 1, "{bar:?} clipped horizontally");
            assert!(y0 > 0 && y1 < bar.height - 1, "{bar:?} clipped vertically");
        }
    }

    #[test]
    fn values_format_by_kind_unit_and_scale() {
        let mut temp = Readout::new(&constant(ReadoutKind::CpuTemp, 47.4), &[]).unwrap();
        temp.sample(Instant::now(), &[], style());
        assert_eq!((temp.label.as_str(), temp.text.as_str()), ("CPU", "47°C"));
        temp.sample(
            Instant::now(),
            &[],
            Style {
                fahrenheit: true,
                ..style()
            },
        );
        assert_eq!(temp.text, "117°F");
        let mut usage = Readout::new(&constant(ReadoutKind::GpuUsage, 99.6), &[]).unwrap();
        usage.sample(Instant::now(), &[], style());
        assert_eq!(usage.text, "100%");
        let mut labelled = Readout::new(
            &ReadoutSlot {
                label: Some("PUMP".into()),
                ..constant(ReadoutKind::Sensor, 42.0)
            },
            &[],
        )
        .unwrap();
        labelled.sample(Instant::now(), &[], style());
        assert_eq!(
            (labelled.label.as_str(), labelled.text.as_str()),
            ("PUMP", "42")
        );
        assert!(Readout::new(&ReadoutSlot::default(), &[]).is_none());
    }

    #[test]
    fn temperatures_turn_orange_then_red_at_the_thresholds() {
        let colour = |value: f32, colors: bool| {
            let mut readout = Readout::new(&constant(ReadoutKind::GpuHotspot, value), &[]).unwrap();
            readout.sample(Instant::now(), &[], Style { colors, ..style() });
            readout.color
        };
        assert_eq!(colour(74.9, true), VALUE_COLOR);
        assert_eq!(colour(75.0, true), WARM_COLOR);
        assert_eq!(colour(85.0, true), HOT_COLOR);
        assert_eq!(colour(95.0, false), VALUE_COLOR);
        let mut usage = Readout::new(&constant(ReadoutKind::CpuUsage, 99.0), &[]).unwrap();
        usage.sample(Instant::now(), &[], style());
        assert_eq!(
            usage.color, VALUE_COLOR,
            "usage was coloured as a temperature"
        );
    }

    #[test]
    fn unreadable_sensors_show_a_placeholder_and_previews_report_text() {
        let failing = ReadoutSlot {
            kind: ReadoutKind::GpuTemp,
            source: Some(SensorSourceConfig::Command {
                cmd: "exit 1".into(),
            }),
            ..Default::default()
        };
        let overlay = TemperatureOverlay {
            slots: Some([constant(ReadoutKind::CpuTemp, 50.0), failing]),
            ..Default::default()
        };
        let previews = preview(&overlay, &[]);
        assert_eq!(previews[0].text, "50°C");
        assert_eq!(previews[1].text, UNAVAILABLE);
        assert_eq!(previews[1].label, "GPU");
    }

    #[test]
    fn previews_never_run_command_sources() {
        let root = tempfile::tempdir().unwrap();
        let marker = root.path().join("ran");
        let overlay = TemperatureOverlay {
            slots: Some([
                ReadoutSlot {
                    kind: ReadoutKind::Sensor,
                    label: Some("CMD".into()),
                    source: Some(SensorSourceConfig::Command {
                        cmd: format!("touch {} && echo 5", marker.display()),
                    }),
                    ..Default::default()
                },
                constant(ReadoutKind::CpuTemp, 40.0),
            ]),
            ..Default::default()
        };
        let previews = preview(&overlay, &[]);
        assert_eq!(previews[0].label, "CMD");
        assert_eq!(previews[0].text, UNAVAILABLE);
        assert_eq!(previews[1].text, "40°C");
        assert!(!marker.exists(), "a preview ran a client-supplied command");
    }
}
