use super::layout::Area;
use super::*;
use image::Rgba;
use lianli_shared::media::SensorSourceConfig;
use lianli_shared::playlist::{
    AlternatePlaylist, ReadoutKind, ReadoutSlot, TemperatureOverlay, TimeWindow, Transition,
};
use std::path::Path;

const BLACK: Rgba<u8> = Rgba([0, 0, 0, 255]);

fn font() -> FontVec {
    crate::fonts::load(
        &Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../templates/assets/neon-us88/JetBrainsMonoNL-Medium.ttf"),
    )
    .unwrap()
}

fn still(path: &Path, rgb: [u8; 3]) -> PlaylistItem {
    image::RgbImage::from_pixel(32, 16, image::Rgb(rgb))
        .save(path)
        .unwrap();
    PlaylistItem::new(path)
}

fn two_frame_gif(path: &Path) -> PlaylistItem {
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
    PlaylistItem::new(path)
}

fn video(path: &Path, seconds: &str) -> PlaylistItem {
    let output = std::process::Command::new("ffmpeg")
        .args([
            "-v",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "testsrc=size=32x16:rate=10",
        ])
        .args(["-t", seconds, "-pix_fmt", "yuv420p"])
        .arg(path)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    PlaylistItem::new(path)
}

fn playlist(config: &PlaylistConfig, fit: ImageFit, canvas: (u32, u32)) -> Playlist {
    Playlist::new(
        config,
        fit,
        canvas,
        10.0,
        &[],
        &PreparationControl::new(false),
    )
    .unwrap()
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

fn current(playlist: &Playlist) -> String {
    playlist
        .active
        .entry
        .as_ref()
        .map(|entry| {
            entry
                .item
                .path
                .file_name()
                .unwrap()
                .to_string_lossy()
                .into_owned()
        })
        .unwrap_or_default()
}

fn render(playlist: &mut Playlist, canvas: (u32, u32)) -> RgbaImage {
    let mut image = RgbaImage::from_pixel(canvas.0, canvas.1, BLACK);
    playlist.draw(&mut image);
    image
}

fn centre(playlist: &mut Playlist, canvas: (u32, u32)) -> [u8; 4] {
    render(playlist, canvas)
        .get_pixel(canvas.0 / 2, canvas.1 / 2)
        .0
}

fn constant_slots(first: f32) -> [ReadoutSlot; 2] {
    [
        ReadoutSlot {
            kind: ReadoutKind::CpuTemp,
            source: Some(SensorSourceConfig::Constant { value: first }),
            ..Default::default()
        },
        ReadoutSlot {
            kind: ReadoutKind::GpuTemp,
            source: Some(SensorSourceConfig::Command {
                cmd: "exit 1".into(),
            }),
            ..Default::default()
        },
    ]
}

fn window_around_now() -> TimeWindow {
    let minute = triggers::local_minute();
    let format = |minute: u32| format!("{:02}:{:02}", (minute / 60) % 24, minute % 60);
    TimeWindow {
        start: format((minute + 1440 - 1) % 1440),
        end: format((minute + 2) % 1440),
    }
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
    let font = font();
    let mut playlist = playlist(&config, ImageFit::Contain, canvas);
    let start = Instant::now();
    assert!(playlist.advance(start, &font));
    assert_eq!(centre(&mut playlist, canvas), [255, 0, 0, 255]);

    wait_for_loader(&playlist);
    assert!(!playlist.advance(start + Duration::from_millis(999), &font));
    assert_eq!(
        current(&playlist),
        "red.png",
        "image left before its hold time"
    );
    assert!(playlist.advance(start + Duration::from_secs(1), &font));
    assert_eq!(current(&playlist), "green.gif");
    assert_eq!(centre(&mut playlist, canvas)[1], 255);

    let gif_start = start + Duration::from_secs(1);
    playlist.advance(gif_start + Duration::from_millis(100), &font);
    assert!(
        playlist.loader.is_none(),
        "next item preloaded while the clip still had time to run"
    );
    playlist.advance(gif_start + Duration::from_millis(200), &font);
    assert_eq!(
        current(&playlist),
        "green.gif",
        "changed before the next item loaded"
    );
    wait_for_loader(&playlist);
    assert!(playlist.advance(gif_start + Duration::from_millis(210), &font));
    assert_eq!(current(&playlist), "blue.png");
    assert_eq!(centre(&mut playlist, canvas), [0, 0, 255, 255]);

    wait_for_loader(&playlist);
    assert!(playlist.advance(gif_start + Duration::from_millis(1210), &font));
    assert_eq!(current(&playlist), "red.png", "playlist did not loop");
}

#[test]
fn item_overrides_change_hold_time_plays_and_fit() {
    let root = tempfile::tempdir().unwrap();
    let config = PlaylistConfig {
        items: vec![
            PlaylistItem {
                seconds: Some(3),
                fit: Some(ImageFit::Cover),
                ..still(&root.path().join("long.png"), [9, 9, 9])
            },
            PlaylistItem {
                plays: Some(3),
                ..two_frame_gif(&root.path().join("thrice.gif"))
            },
            still(&root.path().join("after.png"), [1, 1, 1]),
        ],
        image_seconds: 1,
        ..Default::default()
    };
    let font = font();
    let mut playlist = playlist(&config, ImageFit::Contain, (64, 64));
    assert_eq!(
        playlist.active.area,
        Area::full((64, 64)),
        "fit override ignored"
    );
    let start = Instant::now();
    playlist.advance(start, &font);
    wait_for_loader(&playlist);
    playlist.advance(start + Duration::from_secs(2), &font);
    assert_eq!(
        current(&playlist),
        "long.png",
        "item duration override ignored"
    );
    playlist.advance(start + Duration::from_secs(3), &font);
    assert_eq!(current(&playlist), "thrice.gif");

    let gif = start + Duration::from_secs(3);
    for step in 1..6 {
        playlist.advance(gif + Duration::from_millis(100 * step), &font);
        assert_eq!(
            current(&playlist),
            "thrice.gif",
            "left after {step} frames of 6"
        );
    }
    playlist.advance(gif + Duration::from_millis(600), &font);
    wait_for_loader(&playlist);
    playlist.advance(gif + Duration::from_millis(610), &font);
    assert_eq!(current(&playlist), "after.png");
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
    let font = font();
    let mut looping = playlist(&config, ImageFit::Contain, (64, 64));
    let start = Instant::now();
    looping.advance(start, &font);
    looping.advance(start + Duration::from_millis(500), &font);
    assert_eq!(
        current(&looping),
        "short.gif",
        "short GIF did not loop to the minimum"
    );
    looping.advance(start + Duration::from_secs(1), &font);
    wait_for_loader(&looping);
    looping.advance(start + Duration::from_millis(1010), &font);
    assert_eq!(current(&looping), "after.png");

    let capped = PlaylistConfig {
        max_animation_seconds: 1,
        min_animation_seconds: 0,
        ..config
    };
    let mut capped = playlist(&capped, ImageFit::Contain, (64, 64));
    capped.max_motion = Some(Duration::from_millis(150));
    capped.advance(start, &font);
    wait_for_loader(&capped);
    capped.advance(start + Duration::from_millis(150), &font);
    assert_eq!(
        current(&capped),
        "after.png",
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
    let font = font();
    let mut playlist = playlist(&config, ImageFit::Contain, (64, 64));
    let start = Instant::now();
    assert!(playlist.advance(start, &font));
    assert!(!playlist.advance(start + Duration::from_secs(30), &font));
    assert!(playlist.loader.is_none());
}

#[test]
fn unplayable_items_are_skipped_and_retried_only_after_a_backoff() {
    let root = tempfile::tempdir().unwrap();
    let broken = root.path().join("broken.png");
    std::fs::write(&broken, b"not an image").unwrap();
    let config = PlaylistConfig {
        items: vec![
            PlaylistItem::new(&broken),
            still(&root.path().join("ok.png"), [7, 7, 7]),
            PlaylistItem::new(root.path().join("deleted.png")),
        ],
        image_seconds: 1,
        ..Default::default()
    };
    let font = font();
    let mut playlist = playlist(&config, ImageFit::Contain, (64, 64));
    assert_eq!(current(&playlist), "ok.png");
    assert_eq!(playlist.health[&broken].failures, 1);
    assert!(playlist.health[&broken].reported);
    let start = Instant::now();
    playlist.advance(start, &font);
    wait_for_loader(&playlist);
    playlist.advance(start + Duration::from_secs(1), &font);
    assert_eq!(
        current(&playlist),
        "ok.png",
        "a failed load replaced the screen"
    );
    for _ in 0..3 {
        if let Some(loader) = &playlist.loader {
            assert_ne!(loader.entry.item.path, broken, "retried during its backoff");
        }
        wait_for_loader(&playlist);
        playlist.advance(start + Duration::from_secs(1), &font);
    }
    assert_eq!(
        item_backoff(1),
        Duration::from_secs(30),
        "first backoff changed"
    );
    assert_eq!(item_backoff(3), Duration::from_secs(120));
    assert_eq!(item_backoff(40), MAX_ITEM_BACKOFF);

    let only_broken = PlaylistConfig {
        items: vec![PlaylistItem::new(&broken)],
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
fn startup_gives_up_after_a_bounded_number_of_broken_items() {
    let root = tempfile::tempdir().unwrap();
    let mut items: Vec<PlaylistItem> = (0..MAX_INITIAL_ATTEMPTS + 2)
        .map(|index| {
            let path = root.path().join(format!("broken{index}.png"));
            std::fs::write(&path, b"not an image").unwrap();
            PlaylistItem::new(path)
        })
        .collect();
    items.push(still(&root.path().join("late.png"), [1, 1, 1]));
    let config = PlaylistConfig {
        items,
        ..Default::default()
    };
    assert!(Playlist::new(
        &config,
        ImageFit::Contain,
        (64, 64),
        10.0,
        &[],
        &PreparationControl::new(false)
    )
    .is_err());
}

#[test]
fn crossfade_blends_the_outgoing_and_incoming_items_when_enabled() {
    let root = tempfile::tempdir().unwrap();
    let items = vec![
        still(&root.path().join("white.png"), [255, 255, 255]),
        still(&root.path().join("black.png"), [0, 0, 0]),
    ];
    let canvas = (32, 16);
    let font = font();
    for crossfade in [true, false] {
        let config = PlaylistConfig {
            items: items.clone(),
            image_seconds: 1,
            transition: Transition {
                crossfade,
                crossfade_ms: 400,
            },
            ..Default::default()
        };
        let mut playlist = playlist(&config, ImageFit::Contain, canvas);
        let start = Instant::now();
        playlist.advance(start, &font);
        wait_for_loader(&playlist);
        playlist.advance(start + Duration::from_secs(1), &font);
        assert_eq!(current(&playlist), "black.png");
        playlist.advance(start + Duration::from_millis(1200), &font);
        let middle = centre(&mut playlist, canvas)[0];
        if crossfade {
            assert!((110..=145).contains(&middle), "halfway shade was {middle}");
            assert!(playlist.advance(start + Duration::from_millis(1400), &font));
            assert!(playlist.fade.is_none(), "fade never finished");
        } else {
            assert_eq!(middle, 0, "faded with crossfade turned off");
            assert!(playlist.fade.is_none());
        }
        assert_eq!(centre(&mut playlist, canvas)[0], 0);
    }
}

#[test]
fn pause_freezes_timing_and_next_previous_move_through_history() {
    let root = tempfile::tempdir().unwrap();
    let config = PlaylistConfig {
        items: vec![
            still(&root.path().join("one.png"), [1, 1, 1]),
            still(&root.path().join("two.png"), [2, 2, 2]),
            still(&root.path().join("three.png"), [3, 3, 3]),
        ],
        image_seconds: 2,
        ..Default::default()
    };
    let font = font();
    let mut playlist = playlist(&config, ImageFit::Contain, (64, 64));
    let start = Instant::now();
    playlist.advance(start, &font);
    let status = playlist.command(PlaylistAction::Pause, start + Duration::from_secs(1));
    assert!(status.paused);
    wait_for_loader(&playlist);
    playlist.advance(start + Duration::from_secs(10), &font);
    assert_eq!(current(&playlist), "one.png", "advanced while paused");
    playlist.command(PlaylistAction::Resume, start + Duration::from_secs(10));
    playlist.advance(start + Duration::from_millis(10_900), &font);
    assert_eq!(
        current(&playlist),
        "one.png",
        "pause time was not added back"
    );
    playlist.advance(start + Duration::from_secs(11), &font);
    assert_eq!(current(&playlist), "two.png");

    let status = playlist.command(PlaylistAction::Next, start + Duration::from_secs(11));
    assert_eq!(
        status.current.unwrap().file_name().unwrap(),
        "two.png",
        "next reported a change before it happened"
    );
    wait_for_loader(&playlist);
    playlist.advance(start + Duration::from_secs(11), &font);
    assert_eq!(current(&playlist), "three.png");

    playlist.command(PlaylistAction::Previous, start + Duration::from_secs(11));
    wait_for_loader(&playlist);
    playlist.advance(start + Duration::from_secs(11), &font);
    assert_eq!(current(&playlist), "two.png");
    playlist.command(PlaylistAction::Previous, start + Duration::from_secs(11));
    wait_for_loader(&playlist);
    playlist.advance(start + Duration::from_secs(11), &font);
    assert_eq!(current(&playlist), "one.png");
}

#[test]
fn trimmed_video_plays_only_its_segment() {
    let root = tempfile::tempdir().unwrap();
    let config = PlaylistConfig {
        items: vec![
            PlaylistItem {
                start_seconds: Some(1.0),
                end_seconds: Some(1.5),
                ..video(&root.path().join("long.mp4"), "3")
            },
            still(&root.path().join("after.png"), [5, 5, 5]),
        ],
        ..Default::default()
    };
    let font = font();
    let mut playlist = playlist(&config, ImageFit::Contain, (64, 64));
    let start = Instant::now();
    playlist.advance(start, &font);
    playlist.advance(start + Duration::from_millis(400), &font);
    assert_eq!(current(&playlist), "long.mp4", "trimmed clip ended early");
    playlist.advance(start + Duration::from_millis(500), &font);
    wait_for_loader(&playlist);
    playlist.advance(start + Duration::from_millis(510), &font);
    assert_eq!(
        current(&playlist),
        "after.png",
        "the clip played past its half-second segment"
    );
}

#[test]
fn rotated_video_is_placed_by_its_display_size() {
    let root = tempfile::tempdir().unwrap();
    let plain = video(&root.path().join("plain.mp4"), "0.5");
    let rotated = root.path().join("rotated.mp4");
    let output = std::process::Command::new("ffmpeg")
        .args(["-v", "error", "-y", "-display_rotation", "90", "-i"])
        .arg(&plain.path)
        .args(["-c", "copy"])
        .arg(&rotated)
        .output()
        .unwrap();
    assert!(output.status.success());
    let config = PlaylistConfig {
        items: vec![PlaylistItem::new(rotated)],
        ..Default::default()
    };
    let font = font();
    let mut playlist = playlist(&config, ImageFit::Contain, (64, 64));
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
    assert_eq!(playlist.active.pixels().unwrap().dimensions(), (32, 64));
}

#[test]
fn overlay_draws_readings_only_inside_the_bars() {
    let root = tempfile::tempdir().unwrap();
    let config = PlaylistConfig {
        items: vec![still(&root.path().join("wide.png"), [200, 0, 0])],
        overlay: TemperatureOverlay {
            slots: Some(constant_slots(47.0)),
            ..Default::default()
        },
        ..Default::default()
    };
    let canvas = (128, 256);
    let font = font();
    let mut playlist = playlist(&config, ImageFit::Contain, canvas);
    playlist.advance(Instant::now(), &font);
    let texts: Vec<String> = playlist
        .overlay
        .as_ref()
        .unwrap()
        .readouts
        .iter()
        .flatten()
        .map(|readout| readout.text.clone())
        .collect();
    assert_eq!(texts, ["47°C", readout::UNAVAILABLE]);
    let image = render(&mut playlist, canvas);
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
    let lit = |rows: std::ops::Range<u32>| {
        rows.flat_map(|y| (0..canvas.0).map(move |x| (x, y)))
            .filter(|&(x, y)| image.get_pixel(x, y).0[1] > 0)
            .count()
    };
    assert!(lit(0..96) > 0, "first readout missing from the top bar");
    assert!(
        lit(160..256) > 0,
        "second readout missing from the bottom bar"
    );
    assert!(
        (96..160).all(|y| (0..canvas.0).all(|x| image.get_pixel(x, y).0 == [200, 0, 0, 255])),
        "overlay covered the media"
    );
}

#[test]
fn reserved_space_makes_bars_for_media_that_fills_the_screen() {
    let root = tempfile::tempdir().unwrap();
    let tall = root.path().join("tall.png");
    image::RgbImage::from_pixel(64, 128, image::Rgb([9, 9, 9]))
        .save(&tall)
        .unwrap();
    let mut config = PlaylistConfig {
        items: vec![PlaylistItem::new(&tall)],
        overlay: TemperatureOverlay {
            slots: Some(constant_slots(50.0)),
            ..Default::default()
        },
        ..Default::default()
    };
    let canvas = (128, 256);
    let font = font();
    let mut natural = playlist(&config, ImageFit::Contain, canvas);
    natural.advance(Instant::now(), &font);
    assert_eq!(natural.overlay.as_ref().unwrap().bars, None);
    config.overlay.reserve_px = 64;
    let mut reserved = playlist(&config, ImageFit::Contain, canvas);
    reserved.advance(Instant::now(), &font);
    assert!(reserved.overlay.as_ref().unwrap().bars.is_some());
}

#[test]
fn overlay_hides_when_fill_modes_leave_no_bars() {
    let root = tempfile::tempdir().unwrap();
    let config = PlaylistConfig {
        items: vec![still(&root.path().join("wide.png"), [200, 0, 0])],
        overlay: TemperatureOverlay {
            slots: Some(constant_slots(47.0)),
            ..Default::default()
        },
        ..Default::default()
    };
    let font = font();
    let mut playlist = playlist(&config, ImageFit::Cover, (128, 256));
    playlist.advance(Instant::now(), &font);
    assert_eq!(playlist.overlay.as_ref().unwrap().bars, None);
}

#[test]
fn alternate_readouts_only_takes_over_during_its_window_and_dims() {
    let root = tempfile::tempdir().unwrap();
    let config = PlaylistConfig {
        items: vec![still(&root.path().join("main.png"), [200, 200, 200])],
        overlay: TemperatureOverlay {
            slots: Some(constant_slots(47.0)),
            ..Default::default()
        },
        alternate: Some(AlternatePlaylist {
            readouts_only: true,
            window: Some(window_around_now()),
            ..Default::default()
        }),
        dimming: Some(Dimming {
            window: window_around_now(),
            percent: 50,
        }),
        ..Default::default()
    };
    let canvas = (128, 256);
    let font = font();
    let mut playlist = playlist(&config, ImageFit::Contain, canvas);
    assert!(playlist.advance(Instant::now(), &font));
    assert!(playlist.status().alternate);
    assert!(playlist.status().current.is_none());
    assert_eq!(
        playlist.overlay.as_ref().unwrap().bars,
        Some(halves(canvas))
    );
    let image = render(&mut playlist, canvas);
    let brightest = image.pixels().map(|pixel| pixel.0[0]).max().unwrap();
    assert!(
        (100..=140).contains(&brightest),
        "readouts were not dimmed to half: {brightest}"
    );
}

#[test]
fn alternate_items_play_while_a_listed_program_runs() {
    let root = tempfile::tempdir().unwrap();
    let config = PlaylistConfig {
        items: vec![still(&root.path().join("main.png"), [1, 1, 1])],
        alternate: Some(AlternatePlaylist {
            items: vec![still(&root.path().join("game.png"), [2, 2, 2])],
            processes: vec!["no-such-program-xyz".into()],
            ..Default::default()
        }),
        ..Default::default()
    };
    let font = font();
    let mut playlist = playlist(&config, ImageFit::Contain, (64, 64));
    let start = Instant::now();
    playlist.advance(start, &font);
    assert_eq!(current(&playlist), "main.png");
    playlist.group = Group::Alternate;
    playlist.next_trigger = Some(start + Duration::from_secs(3600));
    playlist.advance(start, &font);
    wait_for_loader(&playlist);
    playlist.advance(start, &font);
    assert_eq!(current(&playlist), "game.png");
    assert!(playlist.status().alternate);
}

#[test]
fn live_folders_are_rescanned_between_passes() {
    let root = tempfile::tempdir().unwrap();
    let folder = root.path().join("live");
    std::fs::create_dir(&folder).unwrap();
    still(&folder.join("a.png"), [1, 1, 1]);
    still(&folder.join("b.png"), [2, 2, 2]);
    let config = PlaylistConfig {
        folders: vec![PlaylistFolder {
            path: folder.clone(),
            recursive: false,
        }],
        image_seconds: 1,
        ..Default::default()
    };
    let font = font();
    let mut playlist = playlist(&config, ImageFit::Contain, (64, 64));
    assert_eq!(current(&playlist), "a.png");
    still(&folder.join("c.png"), [3, 3, 3]);
    let start = Instant::now();
    playlist.next_rescan = Some(start);
    playlist.advance(start, &font);
    let deadline = Instant::now() + Duration::from_secs(5);
    while playlist.rescanned.is_none() {
        assert!(Instant::now() < deadline, "rescan never finished");
        thread::sleep(Duration::from_millis(5));
        playlist.poll_rescan(start);
    }
    let mut seen = vec![current(&playlist)];
    for step in 1..=5 {
        let now = start + Duration::from_secs(step);
        wait_for_loader(&playlist);
        playlist.advance(now, &font);
        seen.push(current(&playlist));
    }
    assert!(
        seen.contains(&"c.png".to_string()),
        "new file never played: {seen:?}"
    );
}

#[test]
fn repeated_frames_of_a_cached_clip_share_a_frame_key() {
    let root = tempfile::tempdir().unwrap();
    let config = PlaylistConfig {
        items: vec![two_frame_gif(&root.path().join("loop.gif"))],
        ..Default::default()
    };
    let font = font();
    let mut playlist = playlist(&config, ImageFit::Contain, (64, 64));
    let start = Instant::now();
    playlist.advance(start, &font);
    let first = playlist.frame_key().expect("cached clip has a key");
    playlist.advance(start + Duration::from_millis(100), &font);
    let second = playlist.frame_key().unwrap();
    assert_ne!(first, second);
    playlist.advance(start + Duration::from_millis(200), &font);
    assert_eq!(
        playlist.frame_key(),
        Some(first),
        "loop did not repeat its key"
    );
}
