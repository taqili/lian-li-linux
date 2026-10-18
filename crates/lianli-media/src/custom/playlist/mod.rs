//! Mixed image, GIF and video playlist rendered through `CustomAsset`, with
//! readouts in the bars Fit mode leaves empty, crossfades, transport controls,
//! an alternate set triggered by programs or time, and scheduled dimming.

mod layout;
mod readout;
mod source;
mod triggers;

pub(crate) use readout::preview as readout_preview;

use ab_glyph::FontVec;
use image::RgbaImage;
use layout::{halves, side_bars};
use lianli_shared::playlist::{
    scan_folder, AlternatePlaylist, Dimming, PlaylistAction, PlaylistConfig, PlaylistFolder,
    PlaylistItem, PlaylistStatus, MAX_ITEMS,
};
use lianli_shared::sensors::SensorInfo;
use lianli_shared::template::ImageFit;
use readout::Overlay;
use source::{load, retire, spawn_load, Content, Entry, Group, Loaded, Loader, Settings, Shuffler};
use std::collections::HashMap;
use std::path::PathBuf;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use tracing::warn;
use triggers::ProcessWatch;

use crate::custom::helpers::fast_overlay;
use crate::{MediaError, PreparationControl};

const ITEM_BACKOFF: Duration = Duration::from_secs(30);
const MAX_ITEM_BACKOFF: Duration = Duration::from_secs(600);
/// Preloading holds a decoder and an ffmpeg process, so it starts only this
/// long before the current item is due to end.
const PRELOAD_LEAD: Duration = Duration::from_secs(3);
/// Bounds startup work when a cause shared by every item, such as a missing
/// ffmpeg, makes each load fail.
const MAX_INITIAL_ATTEMPTS: usize = 8;
const TRIGGER_INTERVAL: Duration = Duration::from_secs(5);
const RESCAN_INTERVAL: Duration = Duration::from_secs(60);
const MAX_HISTORY: usize = 16;

/// Per-item failure state. A failing item is skipped with a growing backoff
/// and warned about once until it plays through again.
#[derive(Default)]
struct Health {
    failures: u32,
    retry_at: Option<Instant>,
    reported: bool,
}

fn item_backoff(failures: u32) -> Duration {
    ITEM_BACKOFF
        .saturating_mul(1 << failures.saturating_sub(1).min(5))
        .min(MAX_ITEM_BACKOFF)
}

/// Explicit items first, then folder files not already listed, up to the
/// playlist limit.
fn main_entries(explicit: &[PlaylistItem], folder_files: &[PathBuf]) -> Vec<Entry> {
    let mut seen = std::collections::HashSet::new();
    explicit
        .iter()
        .cloned()
        .chain(folder_files.iter().cloned().map(PlaylistItem::new))
        .filter(|item| seen.insert(item.path.clone()))
        .filter_map(|item| Entry::new(item, Group::Main))
        .take(MAX_ITEMS)
        .collect()
}

fn scan_folders(folders: &[PlaylistFolder]) -> Vec<PathBuf> {
    let mut files = Vec::new();
    for folder in folders {
        match scan_folder(&folder.path, folder.recursive, MAX_ITEMS) {
            Ok((found, _)) => files.extend(found),
            Err(error) => warn!(
                "playlist folder '{}' cannot be read: {error}",
                folder.path.display()
            ),
        }
    }
    files
}

struct Fade {
    from: Loaded,
    started: Instant,
    duration: Duration,
}

pub(crate) struct Playlist {
    entries: [Vec<Entry>; 2],
    orders: [Vec<usize>; 2],
    cursors: [usize; 2],
    shuffle: Option<Shuffler>,
    settings: Settings,
    image_hold: Duration,
    min_motion: Duration,
    max_motion: Option<Duration>,
    crossfade: Option<Duration>,
    explicit: Vec<PlaylistItem>,
    folders: Vec<PlaylistFolder>,
    rescan: Option<JoinHandle<Vec<PathBuf>>>,
    rescanned: Option<Vec<PathBuf>>,
    next_rescan: Option<Instant>,
    alternate: Option<AlternatePlaylist>,
    watch: Option<ProcessWatch>,
    dimming: Option<Dimming>,
    dim: f32,
    group: Group,
    next_trigger: Option<Instant>,
    active: Loaded,
    active_failed: bool,
    shown_at: Option<Instant>,
    first_pass: Option<Duration>,
    loader: Option<Loader>,
    forced: Option<Entry>,
    control: PreparationControl,
    health: HashMap<PathBuf, Health>,
    idle_until: Option<Instant>,
    overlay: Option<Overlay>,
    paused_at: Option<Instant>,
    skip: bool,
    going_back: bool,
    history: Vec<Entry>,
    fade: Option<Fade>,
    fade_progress: f32,
    fade_buffer: Option<RgbaImage>,
    generation: u64,
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
        let main = main_entries(&config.items, &scan_folders(&config.folders));
        if main.is_empty() {
            return Err(MediaError::InvalidConfig(
                "The playlist's folders contain no playable images, GIFs or videos".into(),
            ));
        }
        let alternate: Vec<Entry> = config
            .alternate
            .iter()
            .flat_map(|alternate| &alternate.items)
            .filter_map(|item| Entry::new(item.clone(), Group::Alternate))
            .collect();
        let mut shuffle = config.shuffle.then(Shuffler::seeded);
        let mut orders = [
            (0..main.len()).collect::<Vec<_>>(),
            (0..alternate.len()).collect::<Vec<_>>(),
        ];
        if let Some(shuffler) = &mut shuffle {
            for order in &mut orders {
                shuffler.shuffle(order, None);
            }
        }
        let settings = Settings {
            canvas,
            fit,
            fps: fps.clamp(1.0, source::MAX_FPS),
            reserve: config.overlay.reserve(),
        };
        let now = Instant::now();
        let mut playlist = Self {
            entries: [main, alternate],
            orders,
            cursors: [0, 0],
            shuffle,
            settings,
            image_hold: config.image_duration(),
            min_motion: config.min_animation_duration(),
            max_motion: config.max_animation_duration(),
            crossfade: config.transition.crossfade_duration(),
            explicit: config.items.clone(),
            folders: config.folders.clone(),
            rescan: None,
            rescanned: None,
            next_rescan: Some(now + RESCAN_INTERVAL),
            watch: config
                .alternate
                .as_ref()
                .and_then(|alternate| ProcessWatch::start(alternate.processes.clone())),
            alternate: config.alternate.clone(),
            dimming: config.dimming.clone(),
            dim: 1.0,
            group: Group::Main,
            next_trigger: None,
            active: Loaded::blank(canvas),
            active_failed: false,
            shown_at: None,
            first_pass: None,
            loader: None,
            forced: None,
            control: PreparationControl::new(control.hardware_video),
            health: HashMap::new(),
            idle_until: None,
            overlay: Overlay::new(&config.overlay, sensors),
            paused_at: None,
            skip: false,
            going_back: false,
            history: Vec::new(),
            fade: None,
            fade_progress: 0.0,
            fade_buffer: None,
            generation: 0,
        };
        playlist.check_triggers(now);
        if !playlist.readouts_only() {
            let group = playlist.group;
            playlist.active = match playlist.first_load(group, control, now) {
                Err(error @ (MediaError::Cancelled | MediaError::HelperTimedOut(_))) => {
                    return Err(error)
                }
                Err(_) if group == Group::Alternate => {
                    playlist.first_load(Group::Main, control, now)?
                }
                result => result?,
            };
        }
        playlist.refresh_bars();
        Ok(playlist)
    }

    fn first_load(
        &mut self,
        group: Group,
        control: &PreparationControl,
        now: Instant,
    ) -> Result<Loaded, MediaError> {
        let index = group as usize;
        let mut last_error = None;
        for position in 0..self.orders[index].len().min(MAX_INITIAL_ATTEMPTS) {
            let entry = &self.entries[index][self.orders[index][position]];
            match load(entry, &self.settings, control) {
                Ok(loaded) => {
                    self.cursors[index] = position + 1;
                    return Ok(loaded);
                }
                Err(error @ (MediaError::Cancelled | MediaError::HelperTimedOut(_))) => {
                    return Err(error)
                }
                Err(error) => {
                    let path = entry.item.path.clone();
                    self.mark_failed(&path, &error, now);
                    last_error = Some(error);
                }
            }
        }
        Err(last_error.unwrap_or(MediaError::EmptyVideo))
    }

    fn readouts_only(&self) -> bool {
        self.group == Group::Alternate
            && self
                .alternate
                .as_ref()
                .is_some_and(|alternate| alternate.readouts_only)
    }

    fn refresh_bars(&mut self) -> bool {
        let canvas = self.settings.canvas;
        let (bars, max_font) = match self.active.content {
            Content::Blank => (Some(halves(canvas)), 360.0),
            _ if self.active.fit == ImageFit::Contain => {
                (side_bars(self.active.area, canvas), 220.0)
            }
            _ => (None, 220.0),
        };
        self.overlay
            .as_mut()
            .is_some_and(|overlay| overlay.set_bars(bars, max_font))
    }

    /// Applies a finished folder rescan between passes so the order of a
    /// pass in progress is never disturbed.
    fn apply_rescan(&mut self) {
        let Some(files) = self.rescanned.take() else {
            return;
        };
        let rebuilt = main_entries(&self.explicit, &files);
        let unchanged = rebuilt.len() == self.entries[0].len()
            && rebuilt
                .iter()
                .zip(&self.entries[0])
                .all(|(a, b)| a.item.path == b.item.path);
        if unchanged || rebuilt.is_empty() {
            return;
        }
        self.orders[0] = (0..rebuilt.len()).collect();
        self.entries[0] = rebuilt;
    }

    fn take_next(&mut self, group: Group) -> Option<Entry> {
        let index = group as usize;
        if self.cursors[index] >= self.orders[index].len() {
            if group == Group::Main {
                self.apply_rescan();
            }
            self.cursors[index] = 0;
            let avoid = self.active.entry.as_ref().and_then(|active| {
                self.entries[index]
                    .iter()
                    .position(|entry| entry.item.path == active.item.path)
            });
            if let Some(shuffler) = &mut self.shuffle {
                shuffler.shuffle(&mut self.orders[index], avoid);
            }
        }
        let entry = self.entries[index]
            .get(*self.orders[index].get(self.cursors[index])?)?
            .clone();
        self.cursors[index] += 1;
        Some(entry)
    }

    /// The next item in order that is not backing off after a failure. When
    /// every item is backing off, waits until the earliest one may retry.
    fn take_next_ready(&mut self, now: Instant) -> Option<Entry> {
        for _ in 0..self.entries[self.group as usize].len() {
            let entry = self.take_next(self.group)?;
            let ready = self
                .health
                .get(&entry.item.path)
                .and_then(|health| health.retry_at)
                .is_none_or(|at| now >= at);
            if ready {
                return Some(entry);
            }
        }
        self.idle_until = self.health.values().filter_map(|h| h.retry_at).min();
        None
    }

    fn hold(&self) -> Duration {
        self.active
            .entry
            .as_ref()
            .and_then(|entry| entry.item.seconds)
            .map_or(self.image_hold, |seconds| {
                Duration::from_secs(u64::from(seconds))
            })
    }

    fn only_item(&self) -> bool {
        let entries = &self.entries[self.group as usize];
        entries.len() == 1
            && self
                .active
                .entry
                .as_ref()
                .is_some_and(|active| active.item.path == entries[0].item.path)
    }

    fn wrong_content(&self) -> bool {
        let blank = matches!(self.active.content, Content::Blank);
        blank != self.readouts_only() || (!blank && self.active.group != self.group)
    }

    fn active_done(&self, now: Instant) -> bool {
        if self.wrong_content() || self.skip || self.active_failed {
            return true;
        }
        if self.paused_at.is_some()
            || matches!(self.active.content, Content::Blank)
            || self.only_item()
        {
            return false;
        }
        let Some(shown_at) = self.shown_at else {
            return false;
        };
        let elapsed = now.saturating_duration_since(shown_at);
        match &self.active.content {
            Content::Still { .. } => elapsed >= self.hold(),
            Content::Motion(_) => {
                let plays = self
                    .active
                    .entry
                    .as_ref()
                    .and_then(|entry| entry.item.plays)
                    .unwrap_or(1);
                self.max_motion.is_some_and(|max| elapsed >= max)
                    || (elapsed >= self.min_motion
                        && self
                            .first_pass
                            .is_some_and(|pass| elapsed >= pass.saturating_mul(plays)))
            }
            Content::Blank => false,
        }
    }

    fn preload_due(&self, now: Instant) -> bool {
        if self.loader.is_some()
            || self.idle_until.is_some_and(|at| now < at)
            || self.readouts_only()
        {
            return false;
        }
        if self.forced.is_some() || self.skip || self.active_failed || self.wrong_content() {
            return true;
        }
        if self.paused_at.is_some() || self.only_item() {
            return false;
        }
        let Some(shown_at) = self.shown_at else {
            return false;
        };
        let elapsed = now.saturating_duration_since(shown_at) + PRELOAD_LEAD;
        match &self.active.content {
            Content::Still { .. } => elapsed >= self.hold(),
            Content::Motion(_) => {
                self.max_motion.is_some_and(|max| elapsed >= max) || self.active_done(now)
            }
            Content::Blank => true,
        }
    }

    fn start_loading(&mut self, now: Instant) {
        if !self.preload_due(now) {
            return;
        }
        self.idle_until = None;
        let Some(entry) = self.forced.take().or_else(|| self.take_next_ready(now)) else {
            return;
        };
        match spawn_load(entry, self.settings, self.control.next_asset()) {
            Ok(loader) => self.loader = Some(loader),
            Err(error) => {
                warn!("playlist could not start loading the next item: {error}");
                self.idle_until = Some(now + ITEM_BACKOFF);
            }
        }
    }

    fn mark_failed(&mut self, path: &std::path::Path, error: &MediaError, now: Instant) {
        if matches!(error, MediaError::Cancelled) {
            return;
        }
        let health = self.health.entry(path.to_path_buf()).or_default();
        health.failures = health.failures.saturating_add(1);
        health.retry_at = Some(now + item_backoff(health.failures));
        if !std::mem::replace(&mut health.reported, true) {
            warn!(
                "playlist item '{}' cannot be played: {error}",
                path.display()
            );
        }
    }

    /// Shows `next`, fading from the current item when crossfades are on.
    fn switch_to(&mut self, next: Loaded, now: Instant, failed: bool) {
        if let Some(entry) = &self.active.entry {
            if !self.active_failed {
                self.health.remove(&entry.item.path);
            }
            if !self.going_back && self.active.group == next.group {
                if self.history.len() == MAX_HISTORY {
                    self.history.remove(0);
                }
                self.history.push(entry.clone());
            }
        }
        let previous = std::mem::replace(&mut self.active, next);
        let fades = self
            .crossfade
            .filter(|_| previous.pixels().is_some() || self.active.pixels().is_some());
        if let Some(old) = self.fade.take() {
            retire(old.from);
        }
        match fades {
            Some(duration) => {
                self.fade = Some(Fade {
                    from: previous,
                    started: now,
                    duration,
                });
                self.fade_progress = 0.0;
            }
            None => retire(previous),
        }
        self.active_failed = failed;
        self.shown_at = Some(now);
        if self.paused_at.is_some() {
            self.paused_at = Some(now);
        }
        self.first_pass = None;
        self.skip = false;
        self.going_back = false;
        self.generation += 1;
        self.refresh_bars();
        self.start_loading(now);
    }

    /// Swaps in the preloaded item once it is ready. Until then the current
    /// item keeps playing, so a slow load never blanks the screen.
    fn try_switch(&mut self, now: Instant) -> bool {
        if self.readouts_only() {
            if matches!(self.active.content, Content::Blank) {
                return false;
            }
            self.switch_to(Loaded::blank(self.settings.canvas), now, false);
            return true;
        }
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
        let mut loaded = match result {
            Ok(loaded) if loaded.group == self.group => loaded,
            Ok(stale) => {
                retire(stale);
                self.start_loading(now);
                return false;
            }
            Err(error) => {
                self.mark_failed(&loader.entry.item.path, &error, now);
                self.start_loading(now);
                return false;
            }
        };
        let started = match &mut loaded.content {
            // Starts the clip's clock on the frame that first shows it.
            Content::Motion(stream) => stream.advance(now).map(|_| ()),
            _ => Ok(()),
        };
        if let Err(error) = &started {
            self.mark_failed(&loader.entry.item.path, error, now);
        }
        self.switch_to(loaded, now, started.is_err());
        true
    }

    fn check_triggers(&mut self, now: Instant) -> bool {
        if self.next_trigger.is_some_and(|at| now < at) {
            return false;
        }
        self.next_trigger = Some(now + TRIGGER_INTERVAL);
        let needs_clock = self.dimming.is_some()
            || self
                .alternate
                .as_ref()
                .is_some_and(|alternate| alternate.window.is_some());
        let minute = needs_clock.then(triggers::local_minute);
        let alternate = self.alternate.as_ref().is_some_and(|alternate| {
            minute.is_some_and(|minute| {
                alternate
                    .window
                    .as_ref()
                    .is_some_and(|window| window.contains(minute))
            }) || self.watch.as_ref().is_some_and(ProcessWatch::running)
        });
        let group = if alternate {
            Group::Alternate
        } else {
            Group::Main
        };
        if group != self.group {
            self.group = group;
            if self.loader.as_ref().is_some_and(|l| l.entry.group != group) {
                // The stale load finishes and drops its result on its own thread.
                self.loader = None;
            }
            self.forced = None;
            self.history.clear();
        }
        let dim = self
            .dimming
            .as_ref()
            .filter(|dimming| minute.is_some_and(|minute| dimming.window.contains(minute)))
            .map_or(1.0, |dimming| f32::from(dimming.percent) / 100.0);
        let dim_changed = dim != self.dim;
        self.dim = dim;
        dim_changed
    }

    fn poll_rescan(&mut self, now: Instant) {
        if self.folders.is_empty() {
            return;
        }
        if self.rescan.as_ref().is_some_and(JoinHandle::is_finished) {
            if let Ok(files) = self.rescan.take().expect("finished rescan").join() {
                self.rescanned = Some(files);
            }
        }
        if self.rescan.is_none() && self.next_rescan.is_none_or(|at| now >= at) {
            self.next_rescan = Some(now + RESCAN_INTERVAL);
            let folders = self.folders.clone();
            self.rescan = thread::Builder::new()
                .name("playlist-rescan".into())
                .spawn(move || scan_folders(&folders))
                .ok();
        }
    }

    fn resume(&mut self, now: Instant) {
        let Some(paused_at) = self.paused_at.take() else {
            return;
        };
        let paused = now.saturating_duration_since(paused_at);
        if let Some(shown_at) = &mut self.shown_at {
            *shown_at += paused;
        }
        if let Content::Motion(stream) = &mut self.active.content {
            stream.shift(paused);
        }
        if let Some(fade) = &mut self.fade {
            fade.started += paused;
        }
    }

    pub(crate) fn command(&mut self, action: PlaylistAction, now: Instant) -> PlaylistStatus {
        match action {
            PlaylistAction::Next => self.skip = true,
            PlaylistAction::Previous => {
                if let Some(previous) = self.history.pop() {
                    // A load already under way finishes and drops its result on its own thread.
                    self.loader = None;
                    self.forced = Some(previous);
                    self.going_back = true;
                    self.skip = true;
                }
            }
            PlaylistAction::Pause => {
                self.paused_at.get_or_insert(now);
            }
            PlaylistAction::Resume => self.resume(now),
            PlaylistAction::TogglePause => {
                if self.paused_at.is_some() {
                    self.resume(now);
                } else {
                    self.paused_at = Some(now);
                }
            }
            PlaylistAction::Status => {}
        }
        self.start_loading(now);
        self.status()
    }

    pub(crate) fn status(&self) -> PlaylistStatus {
        PlaylistStatus {
            paused: self.paused_at.is_some(),
            current: self
                .active
                .entry
                .as_ref()
                .map(|entry| entry.item.path.clone()),
            alternate: self.group == Group::Alternate,
        }
    }

    pub(crate) fn advance(&mut self, now: Instant, font: &FontVec) -> bool {
        let mut changed = self.check_triggers(now);
        self.poll_rescan(now);
        if self.shown_at.is_none() {
            self.shown_at = Some(now);
            changed = true;
        }
        let paused = self.paused_at.is_some();
        if !paused && !self.active_failed {
            if let Content::Motion(stream) = &mut self.active.content {
                match stream.advance(now) {
                    Ok(advanced) => {
                        changed |= advanced;
                        if self.first_pass.is_none() && stream.first_pass_complete(now) {
                            let elapsed = self
                                .shown_at
                                .map_or(Duration::ZERO, |at| now.saturating_duration_since(at));
                            self.first_pass = Some(elapsed.max(Duration::from_millis(1)));
                        }
                    }
                    Err(error) => {
                        self.active_failed = true;
                        if let Some(path) = self.active.entry.as_ref().map(|e| e.item.path.clone())
                        {
                            self.mark_failed(&path, &error, now);
                        }
                    }
                }
            }
        }
        if self.active_done(now) {
            changed |= self.try_switch(now);
        } else {
            self.start_loading(now);
        }
        if let Some(fade) = &self.fade {
            if !paused {
                let progress = now.saturating_duration_since(fade.started).as_secs_f32()
                    / fade.duration.as_secs_f32();
                if progress >= 1.0 {
                    let finished = self.fade.take().expect("fade in progress");
                    retire(finished.from);
                } else {
                    self.fade_progress = progress;
                }
                changed = true;
            }
        }
        if let Some(overlay) = &mut self.overlay {
            changed |= overlay.update(now, font);
        }
        changed
    }

    pub(crate) fn draw(&mut self, canvas: &mut RgbaImage) {
        if let Some(fade) = &self.fade {
            let buffer = self
                .fade_buffer
                .get_or_insert_with(|| RgbaImage::new(canvas.width(), canvas.height()));
            if buffer.dimensions() != canvas.dimensions() {
                *buffer = RgbaImage::new(canvas.width(), canvas.height());
            }
            buffer.copy_from_slice(canvas.as_raw());
            draw_content(buffer, &fade.from);
            draw_content(canvas, &self.active);
            blend(canvas, buffer, self.fade_progress);
        } else {
            draw_content(canvas, &self.active);
        }
        if let Some(overlay) = &self.overlay {
            overlay.draw(canvas);
        }
        if self.dim < 1.0 {
            dim(canvas, self.dim);
        }
    }

    /// Identifies the frame about to be drawn when it is a repeat of an
    /// earlier one, so its encoded output can be reused.
    pub(crate) fn frame_key(&self) -> Option<(u64, u64, u32, usize)> {
        if self.fade.is_some() {
            return None;
        }
        let Content::Motion(stream) = &self.active.content else {
            return None;
        };
        Some((
            self.generation,
            self.overlay.as_ref().map_or(0, |overlay| overlay.version),
            self.dim.to_bits(),
            stream.cache_position()?,
        ))
    }
}

fn draw_content(canvas: &mut RgbaImage, loaded: &Loaded) {
    if let Some(pixels) = loaded.pixels() {
        fast_overlay(
            canvas,
            pixels,
            i64::from(loaded.area.x),
            i64::from(loaded.area.y),
        );
    }
}

/// `canvas = from + (canvas - from) * progress`, in place.
fn blend(canvas: &mut RgbaImage, from: &RgbaImage, progress: f32) {
    let weight = (progress.clamp(0.0, 1.0) * 256.0) as u32;
    for (to, from) in canvas.iter_mut().zip(from.iter()) {
        *to = ((u32::from(*from) * (256 - weight) + u32::from(*to) * weight) >> 8) as u8;
    }
}

fn dim(canvas: &mut RgbaImage, factor: f32) {
    let scale = (factor.clamp(0.0, 1.0) * 256.0) as u32;
    for pixel in canvas.pixels_mut() {
        for channel in &mut pixel.0[..3] {
            *channel = ((u32::from(*channel) * scale) >> 8) as u8;
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
mod tests;
