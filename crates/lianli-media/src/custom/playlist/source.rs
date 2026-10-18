//! Loading playlist files into displayable content.

use super::layout::{place_reserving, Area};
use crate::custom::helpers::fit_image;
use crate::resource_budget::RetainedBudget;
use crate::video::widget_stream::VideoStream;
use crate::{MediaError, PreparationControl};
use image::RgbaImage;
use lianli_shared::playlist::{PlaylistItem, PlaylistItemKind};
use lianli_shared::template::ImageFit;
use std::io::BufReader;
use std::thread::{self, JoinHandle};

pub(super) const MAX_FPS: f32 = 60.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Group {
    Main = 0,
    Alternate = 1,
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct Entry {
    pub item: PlaylistItem,
    pub kind: PlaylistItemKind,
    pub group: Group,
}

impl Entry {
    pub(super) fn new(item: PlaylistItem, group: Group) -> Option<Self> {
        Some(Self {
            kind: item.kind()?,
            item,
            group,
        })
    }
}

pub(super) enum Content {
    Still {
        pixels: RgbaImage,
        _budget: RetainedBudget,
    },
    Motion(VideoStream),
    /// No media: the readouts fill the screen.
    Blank,
}

pub(super) struct Loaded {
    pub area: Area,
    pub fit: ImageFit,
    pub content: Content,
    pub entry: Option<Entry>,
    pub group: Group,
}

impl Loaded {
    pub(super) fn blank(canvas: (u32, u32)) -> Self {
        Self {
            area: Area::full(canvas),
            fit: ImageFit::Stretch,
            content: Content::Blank,
            entry: None,
            group: Group::Alternate,
        }
    }

    pub(super) fn pixels(&self) -> Option<&RgbaImage> {
        match &self.content {
            Content::Still { pixels, .. } => Some(pixels),
            Content::Motion(stream) => Some(stream.frame()),
            Content::Blank => None,
        }
    }
}

#[derive(Clone, Copy)]
pub(super) struct Settings {
    pub canvas: (u32, u32),
    pub fit: ImageFit,
    pub fps: f32,
    pub reserve: Option<u32>,
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

pub(super) fn load(
    entry: &Entry,
    settings: &Settings,
    control: &PreparationControl,
) -> Result<Loaded, MediaError> {
    control.check()?;
    let path = &entry.item.path;
    crate::asset_access::check_file(path)?;
    let fit = entry.item.fit.unwrap_or(settings.fit);
    let content_area = |source| place_reserving(source, settings.canvas, fit, settings.reserve);
    let (area, content) = match entry.kind {
        PlaylistItemKind::Image => {
            let image = crate::image::open_image(path)?;
            control.check()?;
            let area = content_area((image.width(), image.height()));
            let mut budget = RetainedBudget::default();
            budget.reserve(area.width as usize * area.height as usize * 4)?;
            let pixels = fit_image(image, area.width, area.height, content_fit(fit));
            (
                area,
                Content::Still {
                    pixels,
                    _budget: budget,
                },
            )
        }
        PlaylistItemKind::Animation | PlaylistItemKind::Video => {
            let (source, fps) = if entry.kind == PlaylistItemKind::Animation {
                (animation_size(path)?, settings.fps)
            } else {
                (
                    crate::video::ffmpeg::probe_display_size(path, control)?,
                    crate::video::ffmpeg::cap_fps_cancellable(path, settings.fps, control)?,
                )
            };
            let area = content_area(source);
            let stream = VideoStream::with_trim(
                path,
                fps.clamp(1.0, MAX_FPS),
                (area.width, area.height),
                content_fit(fit),
                true,
                entry.item.trim(),
                control,
            )?;
            (area, Content::Motion(stream))
        }
    };
    Ok(Loaded {
        area,
        fit,
        content,
        entry: Some(entry.clone()),
        group: entry.group,
    })
}

pub(super) struct Loader {
    pub entry: Entry,
    pub handle: JoinHandle<Result<Loaded, MediaError>>,
}

pub(super) fn spawn_load(
    entry: Entry,
    settings: Settings,
    control: PreparationControl,
) -> std::io::Result<Loader> {
    let item = entry.clone();
    let handle = thread::Builder::new()
        .name("playlist-load".into())
        .spawn(move || load(&item, &settings, &control))?;
    Ok(Loader { entry, handle })
}

/// Stopping a decoder joins its thread and reaps ffmpeg, which must not stall
/// the renderer at every switch. If no thread can be started it drops inline.
pub(super) fn retire(old: Loaded) {
    if matches!(old.content, Content::Motion(_)) {
        let _ = thread::Builder::new()
            .name("playlist-retire".into())
            .spawn(move || drop(old));
    }
}

/// Small deterministic generator for shuffling; playback order needs no
/// cryptographic quality.
pub(super) struct Shuffler(pub u64);

impl Shuffler {
    pub(super) fn seeded() -> Self {
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
    pub(super) fn shuffle(&mut self, order: &mut [usize], avoid: Option<usize>) {
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
