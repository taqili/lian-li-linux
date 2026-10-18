use crate::media::SensorSourceConfig;
use crate::template::ImageFit;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Several LCDs and saved profiles share a 1024 and 4096 media-reference
/// budget, so the main and alternate items together stay within this.
pub const MAX_ITEMS: usize = 200;
pub const MAX_FOLDERS: usize = 8;
pub const MAX_PROCESSES: usize = 16;
pub const MAX_LABEL_CHARS: usize = 12;
pub const MAX_PLAYS: u32 = 100;
pub const DEFAULT_IMAGE_SECONDS: u32 = 10;
pub const MAX_SECONDS: u32 = 3600;
pub const DEFAULT_CROSSFADE_MS: u32 = 600;
pub const CROSSFADE_MS_RANGE: std::ops::RangeInclusive<u32> = 100..=3000;
pub const RESERVE_PX_RANGE: std::ops::RangeInclusive<u32> = 64..=400;
pub const DEFAULT_WARM_C: f32 = 75.0;
pub const DEFAULT_HOT_C: f32 = 85.0;

pub const IMAGE_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "bmp"];
pub const ANIMATION_EXTENSIONS: &[&str] = &["gif"];
pub const VIDEO_EXTENSIONS: &[&str] = &["mp4", "webm", "mkv", "mov", "avi", "m4v"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaylistItemKind {
    Image,
    Animation,
    Video,
}

impl PlaylistItemKind {
    pub fn from_path(path: &Path) -> Option<Self> {
        let extension = path.extension()?.to_str()?.to_ascii_lowercase();
        let has = |list: &[&str]| list.contains(&extension.as_str());
        if has(IMAGE_EXTENSIONS) {
            Some(Self::Image)
        } else if has(ANIMATION_EXTENSIONS) {
            Some(Self::Animation)
        } else if has(VIDEO_EXTENSIONS) {
            Some(Self::Video)
        } else {
            None
        }
    }
}

/// One playlist file with optional settings that override the playlist's.
/// A bare path string is accepted for configurations written before
/// overrides existed.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PlaylistItem {
    pub path: PathBuf,
    /// Hold time for an image.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seconds: Option<u32>,
    /// How many times a GIF or video plays before the next item.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plays: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fit: Option<ImageFit>,
    /// Video trim, in seconds from the start of the file.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_seconds: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end_seconds: Option<f32>,
}

impl PlaylistItem {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            seconds: None,
            plays: None,
            fit: None,
            start_seconds: None,
            end_seconds: None,
        }
    }

    pub fn kind(&self) -> Option<PlaylistItemKind> {
        PlaylistItemKind::from_path(&self.path)
    }

    /// Start and optional end of the part of a video to play, when trimmed.
    pub fn trim(&self) -> Option<(f32, Option<f32>)> {
        if self.kind() != Some(PlaylistItemKind::Video) {
            return None;
        }
        let start = self.start_seconds.filter(|s| *s > 0.0).unwrap_or(0.0);
        let end = self.end_seconds.filter(|e| *e > 0.0);
        (start > 0.0 || end.is_some()).then_some((start, end))
    }

    fn validate(&self) -> Result<(), String> {
        let name = self.path.display();
        let kind = self.kind().ok_or_else(|| {
            format!("playlist item '{name}' is not a supported image, GIF or video")
        })?;
        if self
            .seconds
            .is_some_and(|s| !(1..=MAX_SECONDS).contains(&s))
        {
            return Err(format!(
                "playlist item '{name}' duration must be between 1 and {MAX_SECONDS} seconds"
            ));
        }
        if self.plays.is_some_and(|p| !(1..=MAX_PLAYS).contains(&p)) {
            return Err(format!(
                "playlist item '{name}' must play between 1 and {MAX_PLAYS} times"
            ));
        }
        let finite = |value: Option<f32>| value.is_none_or(|v| v.is_finite() && v >= 0.0);
        if !finite(self.start_seconds) || !finite(self.end_seconds) {
            return Err(format!(
                "playlist item '{name}' trim times must be positive numbers"
            ));
        }
        if (self.start_seconds.is_some() || self.end_seconds.is_some())
            && kind != PlaylistItemKind::Video
        {
            return Err(format!(
                "playlist item '{name}' can only be trimmed if it is a video"
            ));
        }
        if let Some((start, Some(end))) = self.trim() {
            if end <= start {
                return Err(format!("playlist item '{name}' must end after it starts"));
            }
        }
        Ok(())
    }
}

impl<'de> Deserialize<'de> for PlaylistItem {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct Full {
            path: PathBuf,
            #[serde(default)]
            seconds: Option<u32>,
            #[serde(default)]
            plays: Option<u32>,
            #[serde(default)]
            fit: Option<ImageFit>,
            #[serde(default)]
            start_seconds: Option<f32>,
            #[serde(default)]
            end_seconds: Option<f32>,
        }
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Stored {
            Path(PathBuf),
            Full(Full),
        }
        Ok(match Stored::deserialize(deserializer)? {
            Stored::Path(path) => Self::new(path),
            Stored::Full(full) => Self {
                path: full.path,
                seconds: full.seconds,
                plays: full.plays,
                fit: full.fit,
                start_seconds: full.start_seconds,
                end_seconds: full.end_seconds,
            },
        })
    }
}

/// A folder whose supported files are played and rescanned while it runs.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct PlaylistFolder {
    pub path: PathBuf,
    #[serde(default)]
    pub recursive: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct Transition {
    pub crossfade: bool,
    pub crossfade_ms: u32,
}

impl Default for Transition {
    fn default() -> Self {
        Self {
            crossfade: false,
            crossfade_ms: DEFAULT_CROSSFADE_MS,
        }
    }
}

impl Transition {
    pub fn crossfade_duration(&self) -> Option<Duration> {
        self.crossfade.then(|| {
            let ms = self
                .crossfade_ms
                .clamp(*CROSSFADE_MS_RANGE.start(), *CROSSFADE_MS_RANGE.end());
            Duration::from_millis(u64::from(ms))
        })
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReadoutKind {
    #[default]
    Off,
    CpuTemp,
    GpuTemp,
    GpuHotspot,
    CpuUsage,
    GpuUsage,
    Clock,
    /// Any enumerated sensor, such as a fan speed, chosen through `source`.
    Sensor,
}

impl ReadoutKind {
    pub fn is_temperature(self) -> bool {
        matches!(self, Self::CpuTemp | Self::GpuTemp | Self::GpuHotspot)
    }
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct ReadoutSlot {
    pub kind: ReadoutKind,
    /// Overrides the automatically chosen sensor; required for `Sensor`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<SensorSourceConfig>,
    /// Replaces the default label, such as "CPU".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// Draws the last minute of readings under the value.
    pub sparkline: bool,
}

/// Readouts drawn into the bars that Fit mode leaves beside the media: the
/// first slot on the left or top, the second on the right or bottom.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct TemperatureOverlay {
    /// Superseded by `slots`; still read from older configurations.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub cpu: bool,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub gpu: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cpu_source: Option<SensorSourceConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gpu_source: Option<SensorSourceConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub slots: Option<[ReadoutSlot; 2]>,
    pub fahrenheit: bool,
    /// Colours temperatures at or above `warm_c` orange and `hot_c` red.
    pub colors: bool,
    pub warm_c: f32,
    pub hot_c: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub font_path: Option<PathBuf>,
    /// Shrinks Fit media so bars at least this wide exist; 0 turns it off.
    pub reserve_px: u32,
}

impl Default for TemperatureOverlay {
    fn default() -> Self {
        Self {
            cpu: false,
            gpu: false,
            cpu_source: None,
            gpu_source: None,
            slots: None,
            fahrenheit: false,
            colors: true,
            warm_c: DEFAULT_WARM_C,
            hot_c: DEFAULT_HOT_C,
            font_path: None,
            reserve_px: 0,
        }
    }
}

impl TemperatureOverlay {
    /// The two slots, translating the older CPU and GPU switches when no
    /// slots have been saved.
    pub fn effective_slots(&self) -> [ReadoutSlot; 2] {
        if let Some(slots) = &self.slots {
            return slots.clone();
        }
        let legacy =
            |on: bool, kind: ReadoutKind, source: &Option<SensorSourceConfig>| ReadoutSlot {
                kind: if on { kind } else { ReadoutKind::Off },
                source: source.clone(),
                ..Default::default()
            };
        [
            legacy(self.cpu, ReadoutKind::CpuTemp, &self.cpu_source),
            legacy(self.gpu, ReadoutKind::GpuTemp, &self.gpu_source),
        ]
    }

    pub fn enabled(&self) -> bool {
        self.effective_slots()
            .iter()
            .any(|slot| slot.kind != ReadoutKind::Off)
    }

    pub fn reserve(&self) -> Option<u32> {
        (self.reserve_px > 0 && self.enabled()).then(|| {
            self.reserve_px
                .clamp(*RESERVE_PX_RANGE.start(), *RESERVE_PX_RANGE.end())
        })
    }

    fn validate(&self) -> Result<(), String> {
        for slot in self.effective_slots() {
            if slot.kind == ReadoutKind::Sensor && slot.source.is_none() {
                return Err("a sensor readout needs a sensor to be chosen".into());
            }
            if slot
                .label
                .as_ref()
                .is_some_and(|label| label.chars().count() > MAX_LABEL_CHARS)
            {
                return Err(format!(
                    "readout labels must be at most {MAX_LABEL_CHARS} characters"
                ));
            }
        }
        if !self.warm_c.is_finite() || !self.hot_c.is_finite() || self.hot_c < self.warm_c {
            return Err("the hot temperature must not be below the warm temperature".into());
        }
        if self.reserve_px > 0 && !RESERVE_PX_RANGE.contains(&self.reserve_px) {
            return Err(format!(
                "reserved readout space must be 0 or between {} and {} pixels",
                RESERVE_PX_RANGE.start(),
                RESERVE_PX_RANGE.end()
            ));
        }
        Ok(())
    }
}

/// A daily span of local time, which may cross midnight.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct TimeWindow {
    pub start: String,
    pub end: String,
}

fn minute_of_day(text: &str) -> Option<u32> {
    let (hours, minutes) = text.split_once(':')?;
    let digits = |part: &str| part.bytes().all(|b| b.is_ascii_digit());
    if !(1..=2).contains(&hours.len()) || minutes.len() != 2 || !digits(hours) || !digits(minutes) {
        return None;
    }
    let (hours, minutes): (u32, u32) = (hours.parse().ok()?, minutes.parse().ok()?);
    (hours < 24 && minutes < 60).then_some(hours * 60 + minutes)
}

impl TimeWindow {
    pub fn contains(&self, minute: u32) -> bool {
        let (Some(start), Some(end)) = (minute_of_day(&self.start), minute_of_day(&self.end))
        else {
            return false;
        };
        if start <= end {
            (start..end).contains(&minute)
        } else {
            minute >= start || minute < end
        }
    }

    fn validate(&self) -> Result<(), String> {
        match (minute_of_day(&self.start), minute_of_day(&self.end)) {
            (Some(start), Some(end)) if start != end => Ok(()),
            (Some(_), Some(_)) => {
                Err("a time window must not start and end at the same time".into())
            }
            _ => Err("times must be written as HH:MM".into()),
        }
    }
}

/// A second set of media shown instead of the main items while a listed
/// process runs or during a time window.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct AlternatePlaylist {
    pub items: Vec<PlaylistItem>,
    /// Shows only the readouts, filling the screen, instead of media.
    pub readouts_only: bool,
    pub processes: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub window: Option<TimeWindow>,
}

impl AlternatePlaylist {
    pub fn has_trigger(&self) -> bool {
        !self.processes.is_empty() || self.window.is_some()
    }

    fn validate(&self) -> Result<(), String> {
        if !self.has_trigger() {
            return Err("the alternate playlist needs a program or a time to switch on".into());
        }
        if self.items.is_empty() && !self.readouts_only {
            return Err("the alternate playlist needs items or readouts-only mode".into());
        }
        if self.processes.len() > MAX_PROCESSES
            || self
                .processes
                .iter()
                .any(|name| name.trim().is_empty() || name.len() > 64 || name.contains('/'))
        {
            return Err(format!(
                "list up to {MAX_PROCESSES} program names without paths"
            ));
        }
        if let Some(window) = &self.window {
            window.validate()?;
        }
        self.items.iter().try_for_each(PlaylistItem::validate)
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Dimming {
    pub window: TimeWindow,
    pub percent: u8,
}

/// Images hold for `image_seconds`. GIFs and videos play once; a non-zero
/// `min_animation_seconds` loops short clips until it elapses and a non-zero
/// `max_animation_seconds` cuts long ones off. Item overrides take precedence.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct PlaylistConfig {
    pub items: Vec<PlaylistItem>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub folders: Vec<PlaylistFolder>,
    pub image_seconds: u32,
    pub min_animation_seconds: u32,
    pub max_animation_seconds: u32,
    pub shuffle: bool,
    pub transition: Transition,
    pub overlay: TemperatureOverlay,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alternate: Option<AlternatePlaylist>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dimming: Option<Dimming>,
}

impl Default for PlaylistConfig {
    fn default() -> Self {
        Self {
            items: Vec::new(),
            folders: Vec::new(),
            image_seconds: DEFAULT_IMAGE_SECONDS,
            min_animation_seconds: 0,
            max_animation_seconds: 0,
            shuffle: false,
            transition: Transition::default(),
            overlay: TemperatureOverlay::default(),
            alternate: None,
            dimming: None,
        }
    }
}

impl PlaylistConfig {
    pub fn image_duration(&self) -> Duration {
        Duration::from_secs(u64::from(self.image_seconds.clamp(1, MAX_SECONDS)))
    }

    pub fn min_animation_duration(&self) -> Duration {
        Duration::from_secs(u64::from(self.min_animation_seconds.min(MAX_SECONDS)))
    }

    pub fn max_animation_duration(&self) -> Option<Duration> {
        (self.max_animation_seconds > 0)
            .then(|| Duration::from_secs(u64::from(self.max_animation_seconds.min(MAX_SECONDS))))
    }

    /// Every listed file, main and alternate, in order.
    pub fn all_items(&self) -> impl Iterator<Item = &PlaylistItem> {
        self.items
            .iter()
            .chain(self.alternate.iter().flat_map(|alternate| &alternate.items))
    }

    pub fn all_items_mut(&mut self) -> impl Iterator<Item = &mut PlaylistItem> {
        self.items.iter_mut().chain(
            self.alternate
                .iter_mut()
                .flat_map(|alternate| &mut alternate.items),
        )
    }

    pub fn has_motion(&self) -> bool {
        !self.folders.is_empty()
            || self
                .all_items()
                .any(|item| item.kind() != Some(PlaylistItemKind::Image))
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.items.is_empty() && self.folders.is_empty() {
            return Err("playlist needs at least one image, GIF, video or folder".into());
        }
        if self.all_items().count() > MAX_ITEMS {
            return Err(format!("playlist supports at most {MAX_ITEMS} items"));
        }
        self.items.iter().try_for_each(PlaylistItem::validate)?;
        if self.folders.len() > MAX_FOLDERS {
            return Err(format!("playlist supports at most {MAX_FOLDERS} folders"));
        }
        if let Some(folder) = self.folders.iter().find(|f| !f.path.is_absolute()) {
            return Err(format!(
                "playlist folder '{}' must be an absolute path",
                folder.path.display()
            ));
        }
        if !(1..=MAX_SECONDS).contains(&self.image_seconds) {
            return Err(format!(
                "playlist image duration must be between 1 and {MAX_SECONDS} seconds"
            ));
        }
        if self.min_animation_seconds > MAX_SECONDS || self.max_animation_seconds > MAX_SECONDS {
            return Err(format!(
                "playlist animation durations must be at most {MAX_SECONDS} seconds"
            ));
        }
        if self.max_animation_seconds > 0 && self.max_animation_seconds < self.min_animation_seconds
        {
            return Err(
                "playlist maximum animation duration must not be shorter than the minimum".into(),
            );
        }
        if self.transition.crossfade && !CROSSFADE_MS_RANGE.contains(&self.transition.crossfade_ms)
        {
            return Err(format!(
                "crossfade must last between {} and {} ms",
                CROSSFADE_MS_RANGE.start(),
                CROSSFADE_MS_RANGE.end()
            ));
        }
        self.overlay.validate()?;
        if let Some(alternate) = &self.alternate {
            alternate.validate()?;
            if alternate.readouts_only && !self.overlay.enabled() {
                return Err("readouts-only mode needs at least one readout turned on".into());
            }
        }
        if let Some(dimming) = &self.dimming {
            dimming.window.validate()?;
            if !(10..=100).contains(&dimming.percent) {
                return Err("dimmed brightness must be between 10 and 100 percent".into());
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PlaylistAction {
    Next,
    Previous,
    Pause,
    Resume,
    TogglePause,
    Status,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
pub struct PlaylistStatus {
    pub paused: bool,
    /// The file on screen; absent while only readouts are shown.
    pub current: Option<PathBuf>,
    /// True while the alternate playlist has taken over.
    pub alternate: bool,
}

/// One readout as it would currently appear, for previews.
#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
pub struct ReadoutPreview {
    pub label: String,
    pub text: String,
    /// Display name of the sensor the readout uses, when it reads one.
    pub sensor: Option<String>,
}

/// Supported media under `folder`, sorted by path, at most `limit` files.
/// Hidden entries and symlinked directories are skipped so a scan cannot
/// loop or wander. Returns whether files beyond the limit were left out.
pub fn scan_folder(
    folder: &Path,
    recursive: bool,
    limit: usize,
) -> std::io::Result<(Vec<PathBuf>, bool)> {
    const MAX_DEPTH: usize = 8;
    const MAX_SCANNED: usize = 20_000;
    let mut found = Vec::new();
    let mut scanned = 0;
    let mut truncated = false;
    let mut pending = vec![(folder.to_path_buf(), 0)];
    while let Some((directory, depth)) = pending.pop() {
        let entries = match std::fs::read_dir(&directory) {
            Ok(entries) => entries,
            Err(error) if directory == folder => return Err(error),
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            scanned += 1;
            if scanned > MAX_SCANNED {
                truncated = true;
                break;
            }
            if entry.file_name().to_string_lossy().starts_with('.') {
                continue;
            }
            let path = entry.path();
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            if file_type.is_dir() {
                if recursive && depth < MAX_DEPTH {
                    pending.push((path, depth + 1));
                }
            } else if PlaylistItemKind::from_path(&path).is_some()
                && path.is_file()
                && path.to_str().is_some()
            {
                found.push(path);
            }
        }
    }
    found.sort_by_cached_key(|path| path.to_string_lossy().to_lowercase());
    if found.len() > limit {
        found.truncate(limit);
        truncated = true;
    }
    Ok((found, truncated))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> PlaylistConfig {
        PlaylistConfig {
            items: vec![PlaylistItem::new("/a.png")],
            ..Default::default()
        }
    }

    #[test]
    fn missing_fields_use_documented_defaults() {
        let config: PlaylistConfig =
            serde_json::from_value(serde_json::json!({"items": ["/a.png"]})).unwrap();
        assert_eq!(config.image_duration(), Duration::from_secs(10));
        assert_eq!(config.min_animation_duration(), Duration::ZERO);
        assert_eq!(config.max_animation_duration(), None);
        assert!(!config.shuffle);
        assert_eq!(config.transition.crossfade_duration(), None);
        assert!(!config.overlay.enabled());
        assert!(config.overlay.colors);
        assert!(config.validate().is_ok());
    }

    #[test]
    fn items_accept_plain_paths_and_round_trip_overrides() {
        let config: PlaylistConfig = serde_json::from_value(serde_json::json!({"items": [
            "/plain.png",
            {"path": "/clip.mp4", "plays": 2, "start_seconds": 1.5, "end_seconds": 9, "fit": "cover"}
        ]}))
        .unwrap();
        assert_eq!(config.items[0], PlaylistItem::new("/plain.png"));
        assert_eq!(config.items[1].plays, Some(2));
        assert_eq!(config.items[1].trim(), Some((1.5, Some(9.0))));
        assert_eq!(config.items[1].fit, Some(ImageFit::Cover));
        let encoded = serde_json::to_value(&config).unwrap();
        assert_eq!(
            encoded["items"][0],
            serde_json::json!({"path": "/plain.png"})
        );
        let decoded: PlaylistConfig = serde_json::from_value(encoded).unwrap();
        assert_eq!(decoded.items, config.items);
    }

    #[test]
    fn legacy_cpu_and_gpu_switches_become_slots() {
        let overlay: TemperatureOverlay = serde_json::from_value(serde_json::json!({
            "cpu": true, "gpu_source": {"type": "hwmon", "name": "amdgpu", "label": "temp1"}
        }))
        .unwrap();
        let [first, second] = overlay.effective_slots();
        assert_eq!(first.kind, ReadoutKind::CpuTemp);
        assert_eq!(second.kind, ReadoutKind::Off);
        assert!(overlay.enabled());
        let explicit: TemperatureOverlay = serde_json::from_value(serde_json::json!({
            "cpu": true,
            "slots": [{"kind": "clock"}, {"kind": "gpu_hotspot", "sparkline": true}]
        }))
        .unwrap();
        let [first, second] = explicit.effective_slots();
        assert_eq!(first.kind, ReadoutKind::Clock);
        assert!(second.sparkline);
    }

    #[test]
    fn item_kind_follows_extension_case_insensitively() {
        let kind = |path: &str| PlaylistItemKind::from_path(Path::new(path));
        assert_eq!(kind("/m/photo.JPG"), Some(PlaylistItemKind::Image));
        assert_eq!(kind("/m/loop.gif"), Some(PlaylistItemKind::Animation));
        assert_eq!(kind("/m/clip.MKV"), Some(PlaylistItemKind::Video));
        assert_eq!(kind("/m/notes.txt"), None);
        assert_eq!(kind("/m/no-extension"), None);
    }

    #[test]
    fn validation_rejects_unplayable_or_contradictory_settings() {
        assert!(PlaylistConfig::default().validate().is_err());
        let with = |edit: fn(&mut PlaylistConfig)| {
            let mut config = base();
            edit(&mut config);
            config.validate()
        };
        assert!(with(|c| c.items[0].path = "/a.txt".into())
            .unwrap_err()
            .contains("/a.txt"));
        assert!(with(|c| c.image_seconds = 0).is_err());
        assert!(with(|c| {
            c.min_animation_seconds = 30;
            c.max_animation_seconds = 10;
        })
        .is_err());
        assert!(with(|c| c.min_animation_seconds = 30).is_ok());
        assert!(
            with(|c| c.items[0].start_seconds = Some(1.0)).is_err(),
            "trimmed an image"
        );
        assert!(with(|c| {
            c.items[0] = PlaylistItem {
                start_seconds: Some(5.0),
                end_seconds: Some(5.0),
                ..PlaylistItem::new("/v.mp4")
            }
        })
        .is_err());
        assert!(with(|c| c.items[0].plays = Some(0)).is_err());
        assert!(with(|c| {
            c.transition.crossfade = true;
            c.transition.crossfade_ms = 50;
        })
        .is_err());
        assert!(with(|c| c.overlay.reserve_px = 10).is_err());
        assert!(with(|c| {
            c.overlay.warm_c = 90.0;
            c.overlay.hot_c = 80.0;
        })
        .is_err());
        assert!(with(|c| {
            c.overlay.slots = Some([
                ReadoutSlot {
                    kind: ReadoutKind::Sensor,
                    ..Default::default()
                },
                ReadoutSlot::default(),
            ])
        })
        .is_err());
        assert!(with(|c| c.folders.push(PlaylistFolder {
            path: "relative".into(),
            recursive: false
        }))
        .is_err());
        assert!(with(|c| {
            c.items.clear();
            c.folders.push(PlaylistFolder {
                path: "/media".into(),
                recursive: true,
            });
        })
        .is_ok());
        assert!(
            with(|c| c.alternate = Some(AlternatePlaylist {
                readouts_only: true,
                ..Default::default()
            }))
            .is_err(),
            "alternate without a trigger"
        );
        assert!(
            with(|c| c.alternate = Some(AlternatePlaylist {
                readouts_only: true,
                processes: vec!["/usr/bin/game".into()],
                ..Default::default()
            }))
            .is_err(),
            "process given as a path"
        );
        assert!(with(|c| c.dimming = Some(Dimming {
            window: TimeWindow {
                start: "22:00".into(),
                end: "07:00".into()
            },
            percent: 5
        }))
        .is_err());
    }

    #[test]
    fn time_windows_may_cross_midnight() {
        let night = TimeWindow {
            start: "22:30".into(),
            end: "07:00".into(),
        };
        assert!(night.contains(23 * 60));
        assert!(night.contains(6 * 60 + 59));
        assert!(!night.contains(7 * 60));
        assert!(!night.contains(12 * 60));
        let day = TimeWindow {
            start: "09:00".into(),
            end: "17:00".into(),
        };
        assert!(day.contains(9 * 60));
        assert!(!day.contains(17 * 60));
        assert!(TimeWindow {
            start: "25:00".into(),
            end: "07:00".into()
        }
        .validate()
        .is_err());
        assert_eq!(minute_of_day("9:05"), Some(9 * 60 + 5));
        for bad in ["9:5", "09:5", "+9:05", "009:05", "09:05 ", ":05"] {
            assert_eq!(minute_of_day(bad), None, "{bad:?} was accepted");
        }
    }

    #[test]
    fn folder_scan_sorts_supported_media_and_skips_hidden_or_linked_folders() {
        let root = tempfile::tempdir().unwrap();
        let base = root.path();
        for name in ["b.PNG", "a.mp4", "notes.txt", ".hidden.gif", "c.gif"] {
            std::fs::write(base.join(name), b"x").unwrap();
        }
        std::fs::create_dir(base.join("Nested")).unwrap();
        std::fs::write(base.join("Nested/d.jpg"), b"x").unwrap();
        std::fs::create_dir(base.join(".cache")).unwrap();
        std::fs::write(base.join(".cache/e.png"), b"x").unwrap();
        std::os::unix::fs::symlink(base, base.join("loop")).unwrap();
        let names = |found: Vec<PathBuf>| {
            found
                .iter()
                .map(|path| {
                    path.strip_prefix(base)
                        .unwrap()
                        .to_string_lossy()
                        .into_owned()
                })
                .collect::<Vec<_>>()
        };
        let (flat, truncated) = scan_folder(base, false, MAX_ITEMS).unwrap();
        assert_eq!(names(flat), ["a.mp4", "b.PNG", "c.gif"]);
        assert!(!truncated);
        let (deep, _) = scan_folder(base, true, MAX_ITEMS).unwrap();
        assert_eq!(names(deep), ["a.mp4", "b.PNG", "c.gif", "Nested/d.jpg"]);
        let (capped, truncated) = scan_folder(base, true, 2).unwrap();
        assert_eq!(names(capped), ["a.mp4", "b.PNG"]);
        assert!(truncated);
        assert!(scan_folder(&base.join("missing"), false, 10).is_err());
    }
}
