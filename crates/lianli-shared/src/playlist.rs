use crate::media::SensorSourceConfig;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::Duration;

pub const MAX_ITEMS: usize = 500;
pub const DEFAULT_IMAGE_SECONDS: u32 = 10;
pub const MAX_SECONDS: u32 = 3600;

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

/// Images hold for `image_seconds`. GIFs and videos play once; a non-zero
/// `min_animation_seconds` loops short clips until it elapses and a non-zero
/// `max_animation_seconds` cuts long ones off.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct PlaylistConfig {
    pub items: Vec<PathBuf>,
    pub image_seconds: u32,
    pub min_animation_seconds: u32,
    pub max_animation_seconds: u32,
    pub shuffle: bool,
    pub overlay: TemperatureOverlay,
}

impl Default for PlaylistConfig {
    fn default() -> Self {
        Self {
            items: Vec::new(),
            image_seconds: DEFAULT_IMAGE_SECONDS,
            min_animation_seconds: 0,
            max_animation_seconds: 0,
            shuffle: false,
            overlay: TemperatureOverlay::default(),
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

    pub fn validate(&self) -> Result<(), String> {
        if self.items.is_empty() {
            return Err("playlist needs at least one image, GIF or video".into());
        }
        if self.items.len() > MAX_ITEMS {
            return Err(format!("playlist supports at most {MAX_ITEMS} items"));
        }
        if let Some(path) = self
            .items
            .iter()
            .find(|path| PlaylistItemKind::from_path(path).is_none())
        {
            return Err(format!(
                "playlist item '{}' is not a supported image, GIF or video",
                path.display()
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
        Ok(())
    }
}

/// CPU and GPU temperatures drawn into the bars that Fit mode leaves beside
/// the media. A missing source is picked automatically from the machine's sensors.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct TemperatureOverlay {
    pub cpu: bool,
    pub gpu: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cpu_source: Option<SensorSourceConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gpu_source: Option<SensorSourceConfig>,
}

impl TemperatureOverlay {
    pub fn enabled(&self) -> bool {
        self.cpu || self.gpu
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_fields_use_documented_defaults() {
        let config: PlaylistConfig =
            serde_json::from_value(serde_json::json!({"items": ["/a.png"]})).unwrap();
        assert_eq!(config.image_duration(), Duration::from_secs(10));
        assert_eq!(config.min_animation_duration(), Duration::ZERO);
        assert_eq!(config.max_animation_duration(), None);
        assert!(!config.shuffle);
        assert!(!config.overlay.enabled());
        assert!(config.validate().is_ok());
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
        let base = PlaylistConfig {
            items: vec!["/a.png".into()],
            ..Default::default()
        };
        assert!(PlaylistConfig::default().validate().is_err());
        let unsupported = PlaylistConfig {
            items: vec!["/a.txt".into()],
            ..base.clone()
        };
        assert!(unsupported.validate().unwrap_err().contains("/a.txt"));
        let zero_hold = PlaylistConfig {
            image_seconds: 0,
            ..base.clone()
        };
        assert!(zero_hold.validate().is_err());
        let inverted = PlaylistConfig {
            min_animation_seconds: 30,
            max_animation_seconds: 10,
            ..base.clone()
        };
        assert!(inverted.validate().is_err());
        let unlimited_max = PlaylistConfig {
            min_animation_seconds: 30,
            max_animation_seconds: 0,
            ..base
        };
        assert!(unlimited_max.validate().is_ok());
    }
}
