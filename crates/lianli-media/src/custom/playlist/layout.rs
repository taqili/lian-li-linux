//! Where media and readouts land on the canvas.

use lianli_shared::template::ImageFit;

/// Narrowest bar that still fits a legible two-line readout. A 16:9 clip on a
/// 1472×720 panel leaves 96px on each side.
pub(crate) const MIN_BAR: u32 = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Area {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

impl Area {
    pub(crate) fn full(canvas: (u32, u32)) -> Self {
        Self {
            x: 0,
            y: 0,
            width: canvas.0.max(1),
            height: canvas.1.max(1),
        }
    }
}

/// Where a source of `source` pixels lands on the canvas. Contain keeps the
/// aspect ratio inside the canvas; Cover and Stretch fill it.
pub(crate) fn place(source: (u32, u32), canvas: (u32, u32), fit: ImageFit) -> Area {
    let (cw, ch) = (canvas.0.max(1), canvas.1.max(1));
    if fit != ImageFit::Contain {
        return Area::full(canvas);
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

/// Like [`place`], but when Fit would leave no bar of at least `reserve`
/// pixels, shrinks the media along whichever axis loses the least area so
/// that bars of that width exist.
pub(crate) fn place_reserving(
    source: (u32, u32),
    canvas: (u32, u32),
    fit: ImageFit,
    reserve: Option<u32>,
) -> Area {
    let natural = place(source, canvas, fit);
    let Some(reserve) = reserve.filter(|_| fit == ImageFit::Contain) else {
        return natural;
    };
    let reserve = reserve.max(MIN_BAR);
    if side_bars(natural, canvas).is_some_and(|[bar, _]| bar.width.min(bar.height) >= reserve) {
        return natural;
    }
    let (cw, ch) = canvas;
    let shrink = |inner: (u32, u32), offset: (u32, u32)| {
        (inner.0 > 0 && inner.1 > 0).then(|| {
            let area = place(source, inner, ImageFit::Contain);
            Area {
                x: area.x + offset.0,
                y: area.y + offset.1,
                ..area
            }
        })
    };
    let beside = shrink((cw.saturating_sub(2 * reserve), ch), (reserve, 0));
    let around = shrink((cw, ch.saturating_sub(2 * reserve)), (0, reserve));
    let size = |area: &Area| u64::from(area.width) * u64::from(area.height);
    match (beside, around) {
        (Some(a), Some(b)) => {
            if size(&a) >= size(&b) {
                a
            } else {
                b
            }
        }
        (Some(a), None) | (None, Some(a)) => a,
        (None, None) => natural,
    }
}

/// The pair of empty bars beside or above and below the media: left/top for
/// the first readout and right/bottom for the second. `None` when neither
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

/// The screen split in two along its longer side, for readouts without media.
pub(crate) fn halves(canvas: (u32, u32)) -> [Area; 2] {
    let (cw, ch) = (canvas.0.max(2), canvas.1.max(2));
    if ch >= cw {
        let top = ch / 2;
        [
            Area {
                x: 0,
                y: 0,
                width: cw,
                height: top,
            },
            Area {
                x: 0,
                y: top,
                width: cw,
                height: ch - top,
            },
        ]
    } else {
        let left = cw / 2;
        [
            Area {
                x: 0,
                y: 0,
                width: left,
                height: ch,
            },
            Area {
                x: left,
                y: 0,
                width: cw - left,
                height: ch,
            },
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PORTRAIT: (u32, u32) = (720, 1472);
    const LANDSCAPE: (u32, u32) = (1472, 720);

    fn area(x: u32, y: u32, width: u32, height: u32) -> Area {
        Area {
            x,
            y,
            width,
            height,
        }
    }

    #[test]
    fn contain_keeps_aspect_ratio_and_centres_media() {
        assert_eq!(
            place((1920, 1080), PORTRAIT, ImageFit::Contain),
            area(0, 533, 720, 405)
        );
        assert_eq!(
            place((1920, 1080), LANDSCAPE, ImageFit::Contain),
            area(96, 0, 1280, 720)
        );
        assert_eq!(
            place((1920, 1080), PORTRAIT, ImageFit::Cover),
            area(0, 0, 720, 1472)
        );
    }

    #[test]
    fn readouts_use_whichever_bars_fit_mode_leaves() {
        let beside = side_bars(place((1920, 1080), LANDSCAPE, ImageFit::Contain), LANDSCAPE)
            .expect("16:9 on the landscape panel leaves 96px side bars");
        assert_eq!(beside, [area(0, 0, 96, 720), area(1376, 0, 96, 720)]);
        let stacked = side_bars(place((1920, 1080), PORTRAIT, ImageFit::Contain), PORTRAIT)
            .expect("16:9 on the portrait panel leaves top and bottom bars");
        assert_eq!(stacked, [area(0, 0, 720, 533), area(0, 938, 720, 534)]);
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
    fn reserving_space_shrinks_media_that_would_fill_the_screen() {
        let portrait_media = (720, 1472);
        let reserved = place_reserving(portrait_media, PORTRAIT, ImageFit::Contain, Some(120));
        let bars = side_bars(reserved, PORTRAIT).expect("reserved bars");
        assert!(bars.iter().all(|bar| bar.width.min(bar.height) >= 120));
        let aspect = f64::from(reserved.width) / f64::from(reserved.height);
        assert!(
            (aspect - 720.0 / 1472.0).abs() < 0.005,
            "media was distorted"
        );

        let natural = place((1920, 1080), PORTRAIT, ImageFit::Contain);
        assert_eq!(
            place_reserving((1920, 1080), PORTRAIT, ImageFit::Contain, Some(120)),
            natural,
            "media that already leaves wide bars moved"
        );
        assert_eq!(
            place_reserving(portrait_media, PORTRAIT, ImageFit::Cover, Some(120)),
            Area::full(PORTRAIT)
        );
    }

    #[test]
    fn halves_split_along_the_longer_side() {
        assert_eq!(
            halves(PORTRAIT),
            [area(0, 0, 720, 736), area(0, 736, 720, 736)]
        );
        assert_eq!(
            halves(LANDSCAPE),
            [area(0, 0, 736, 720), area(736, 0, 736, 720)]
        );
    }
}
