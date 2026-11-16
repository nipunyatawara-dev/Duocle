#[cfg(any(target_os = "macos", target_os = "windows"))]
use crate::config::AppInfo;
use crate::config::{Displays, Settings};
use std::{
    collections::HashSet,
    time::{Duration, Instant},
};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Rect {
    pub fn new(x: i32, y: i32, w: i32, h: i32) -> Self {
        Self { x, y, w, h }
    }

    pub fn right(self) -> i32 {
        self.x + self.w
    }

    pub fn bottom(self) -> i32 {
        self.y + self.h
    }

    pub fn intersection(self, other: Self) -> Option<Self> {
        let x = self.x.max(other.x);
        let y = self.y.max(other.y);
        let right = self.right().min(other.right());
        let bottom = self.bottom().min(other.bottom());
        (right > x && bottom > y).then(|| Self::new(x, y, right - x, bottom - y))
    }

    pub fn contains(self, x: i32, y: i32) -> bool {
        x >= self.x && y >= self.y && x < self.right() && y < self.bottom()
    }

    fn subtract(self, other: Self, output: &mut Vec<Self>) {
        let Some(overlap) = self.intersection(other) else {
            output.push(self);
            return;
        };
        let right = overlap.right();
        let bottom = overlap.bottom();
        output.extend(
            [
                Self::new(self.x, self.y, self.w, overlap.y - self.y),
                Self::new(self.x, bottom, self.w, self.bottom() - bottom),
                Self::new(self.x, overlap.y, overlap.x - self.x, overlap.h),
                Self::new(right, overlap.y, self.right() - right, overlap.h),
            ]
            .into_iter()
            .filter(|rect| rect.w > 0 && rect.h > 0),
        );
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Display {
    pub id: u64,
    pub bounds: Rect,
    pub work: Rect,
    pub stage: Option<Rect>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Window {
    pub id: u64,
    pub app: String,
    pub bounds: Rect,
    pub focusable: bool,
    pub corner_radius: i32,
}

#[derive(Default)]
pub struct Desktop {
    pub displays: Vec<Display>,
    /// Windows in front-to-back order.
    pub windows: Vec<Window>,
    pub foreground: Option<u64>,
    pub foreground_app: Option<String>,
    pub overview: bool,
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    pub apps: Vec<AppInfo>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Mask {
    pub bounds: Rect,
    /// Nonoverlapping rectangles within the overlay, in desktop coordinates.
    pub clear: Vec<Rect>,
    pub ambient: Vec<Rect>,
    #[cfg(target_os = "macos")]
    pub behind: Option<u64>,
}

impl Mask {
    pub fn transparent(bounds: Rect) -> Self {
        Self {
            bounds,
            clear: vec![bounds],
            ambient: Vec::new(),
            #[cfg(target_os = "macos")]
            behind: None,
        }
    }

    fn covered_rectangles(&self) -> Vec<Rect> {
        coalesce(uncovered(self.bounds, &self.clear))
    }
}

#[derive(Clone, Copy)]
pub struct Tile {
    pub rect: Rect,
    from: f32,
    to: f32,
}

impl Tile {
    pub fn opacity(self, progress: f32) -> f32 {
        self.from + (self.to - self.from) * progress
    }

    pub fn is_animating(self) -> bool {
        self.from != self.to
    }
}

/// A tile is either already covered, fading in, or fading out. Disjoint tiles
/// keep the wallpaper at the same brightness during a focus switch.
pub struct Coverage {
    pub target: Mask,
    pub tiles: Vec<Tile>,
    start: Instant,
    duration: Duration,
}

impl Coverage {
    pub fn new(target: Mask, now: Instant) -> Self {
        let tiles = target
            .covered_rectangles()
            .into_iter()
            .map(|rect| Tile {
                rect,
                from: 1.0,
                to: 1.0,
            })
            .collect();
        Self {
            target,
            tiles,
            start: now,
            duration: Duration::ZERO,
        }
    }

    pub fn progress(&self, now: Instant) -> f32 {
        if self.duration.is_zero() {
            return 1.0;
        }
        ease(now.saturating_duration_since(self.start).as_secs_f32() / self.duration.as_secs_f32())
    }

    /// Returns true when native view geometry must be updated.
    pub fn update(&mut self, target: &Mask, now: Instant, duration: Duration) -> bool {
        if self.target == *target {
            if !self.duration.is_zero() && (duration.is_zero() || now >= self.start + self.duration)
            {
                *self = Self::new(target.clone(), now);
                return true;
            }
            return false;
        }
        if self.target.bounds != target.bounds || duration.is_zero() {
            *self = Self::new(target.clone(), now);
            return true;
        }
        let progress = self.progress(now);
        let current: Vec<_> = self
            .tiles
            .iter()
            .filter_map(|tile| {
                let opacity = tile.opacity(progress);
                (opacity > 0.0).then_some((tile.rect, opacity))
            })
            .collect();
        let before: Vec<_> = current.iter().map(|(rect, _)| *rect).collect();
        let after = target.covered_rectangles();
        let mut tiles = Vec::new();
        for (rect, opacity) in current {
            tiles.extend(
                after
                    .iter()
                    .filter_map(|next| rect.intersection(*next))
                    .map(|rect| Tile {
                        rect,
                        from: opacity,
                        to: 1.0,
                    }),
            );
            tiles.extend(uncovered(rect, &after).into_iter().map(|rect| Tile {
                rect,
                from: opacity,
                to: 0.0,
            }));
        }
        for next in &after {
            tiles.extend(uncovered(*next, &before).into_iter().map(|rect| Tile {
                rect,
                from: 0.0,
                to: 1.0,
            }));
        }
        if !tiles.iter().any(|tile| tile.is_animating()) {
            *self = Self::new(target.clone(), now);
            return true;
        }
        self.target = target.clone();
        tiles.sort_unstable_by_key(|tile| {
            (
                tile.from.to_bits(),
                tile.to.to_bits(),
                tile.rect.x,
                tile.rect.w,
                tile.rect.y,
            )
        });
        self.tiles.clear();
        for tile in tiles {
            if let Some(last) = self.tiles.last_mut()
                && last.from == tile.from
                && last.to == tile.to
                && last.rect.x == tile.rect.x
                && last.rect.w == tile.rect.w
                && last.rect.bottom() == tile.rect.y
            {
                last.rect.h += tile.rect.h;
            } else {
                self.tiles.push(tile);
            }
        }
        self.start = now;
        self.duration = duration;
        true
    }
}

pub fn ease(progress: f32) -> f32 {
    let progress = progress.clamp(0.0, 1.0);
    progress * progress * (3.0 - 2.0 * progress)
}

fn coalesce(mut rectangles: Vec<Rect>) -> Vec<Rect> {
    // Join runs with identical extents so curved corners don't fragment
    // either of the long, straight sides into native blur views.
    rectangles.sort_unstable_by_key(|r| (r.x, r.w, r.y));
    let mut merged: Vec<Rect> = Vec::with_capacity(rectangles.len());
    for rect in rectangles {
        if let Some(last) = merged.last_mut()
            && last.x == rect.x
            && last.w == rect.w
            && last.bottom() == rect.y
        {
            last.h += rect.h;
        } else {
            merged.push(rect);
        }
    }
    merged
}

fn window_bands(bounds: Rect, radius: i32) -> Vec<Rect> {
    let radius = radius.min(bounds.w / 2).min(bounds.h / 2);
    if radius <= 0 {
        return vec![bounds];
    }
    let power = if cfg!(target_os = "macos") {
        2.8_f32
    } else {
        2.0
    };
    let mut bands = Vec::new();
    for row in 0..radius {
        let height = 1.0 - (row as f32 + 0.5) / radius as f32;
        let reach = (1.0 - height.powf(power)).powf(1.0 / power);
        let inset = ((1.0 - reach) * radius as f32).round() as i32;
        bands.push(Rect::new(
            bounds.x + inset,
            bounds.y + row,
            bounds.w - 2 * inset,
            1,
        ));
    }
    let bottom = bands
        .iter()
        .rev()
        .map(|band| Rect::new(band.x, bounds.bottom() - (band.y - bounds.y) - 1, band.w, 1))
        .collect::<Vec<_>>();
    if bounds.h > 2 * radius {
        bands.push(Rect::new(
            bounds.x,
            bounds.y + radius,
            bounds.w,
            bounds.h - 2 * radius,
        ));
    }
    bands.extend(bottom);
    coalesce(bands)
}

/// A layer-zero preview can be above the document that actually has focus.
#[cfg(target_os = "macos")]
pub fn mac_foreground<'a>(windows: &'a [Window], app: &str) -> Option<&'a Window> {
    let candidates = || {
        windows
            .iter()
            .filter(|window| window.app == app && window.focusable)
    };
    candidates()
        .find(|window| window.bounds.w >= 220 && window.bounds.h >= 150)
        .or_else(|| candidates().next())
        .or_else(|| windows.iter().find(|window| window.app == app))
}

pub fn plan(desktop: &Desktop, settings: &Settings) -> Vec<(u64, Mask)> {
    if desktop.overview
        || desktop
            .foreground_app
            .as_deref()
            .is_some_and(|app| settings.is_ignored(app))
    {
        return Vec::new();
    }

    let foreground = desktop
        .foreground
        .and_then(|id| desktop.windows.iter().find(|window| window.id == id));
    if foreground.is_none() {
        return Vec::new();
    }

    desktop
        .displays
        .iter()
        .filter_map(|display| {
            // On macOS the overlay can fill the display beneath the menu bar and Dock.
            let bounds = if cfg!(target_os = "macos") || settings.dim_chrome {
                display.bounds
            } else {
                display.work
            };
            let primary = match settings.displays {
                Displays::Global => foreground,
                Displays::PerDisplay => foreground
                    .filter(|window| window.bounds.intersection(bounds).is_some())
                    .or_else(|| {
                        desktop.windows.iter().find(|window| {
                            window.focusable && window.bounds.intersection(bounds).is_some()
                        })
                    }),
            };
            let active_app = primary.map(|window| window.app.as_str());
            let focused_bands = primary.map(|window| {
                let radius = if window.bounds == display.bounds {
                    0
                } else {
                    window.corner_radius
                };
                window_bands(window.bounds, radius)
            });
            let mut active_group = HashSet::new();
            if let Some(app) = active_app {
                for group in &settings.groups {
                    if group.apps.iter().any(|member| member.id == app) {
                        active_group.extend(group.apps.iter().map(|member| member.id.as_str()));
                    }
                }
            }

            let mut clear = Vec::new();
            let mut ambient = Vec::new();
            let mut covered = Vec::new();
            if !settings.cover_stage
                && let Some(stage) = display.stage.and_then(|rect| rect.intersection(bounds))
            {
                clear.push(stage);
                ambient.push(stage);
                covered.push(stage);
            }
            for window in &desktop.windows {
                let Some(rect) = window.bounds.intersection(bounds) else {
                    continue;
                };
                let is_clear = primary
                    .is_some_and(|window_in_focus| window.id == window_in_focus.id)
                    || (settings.app_wide && active_app == Some(window.app.as_str()))
                    || active_group.contains(window.app.as_str())
                    || settings.is_ignored(&window.app);
                let square = window.bounds == display.bounds;
                let radius = if square { 0 } else { window.corner_radius };
                let bands = window_bands(window.bounds, radius);
                let same_app = active_app == Some(window.app.as_str());
                let first_clear = clear.len();
                for band in bands
                    .into_iter()
                    .filter_map(|band| band.intersection(bounds))
                {
                    if is_clear {
                        clear.extend(uncovered(band, &covered));
                    } else if same_app && let Some(clips) = &focused_bands {
                        // Keep popovers inside the focused silhouette sharp.
                        for clip in clips {
                            if let Some(part) = clip.intersection(band) {
                                clear.extend(uncovered(part, &covered));
                            }
                        }
                    }
                    covered.push(band);
                }
                if clear.len() != first_clear {
                    ambient.push(rect);
                }
                if radius == 0 && rect == bounds {
                    break;
                }
            }
            #[cfg(target_os = "macos")]
            let behind = primary
                .filter(|primary| {
                    primary.focusable
                        && !settings.dim_chrome
                        && !settings.uses_capture()
                        && !settings.app_wide
                        && active_group.is_empty()
                        && (settings.cover_stage || display.stage.is_none())
                        && front_window_can_cover_overlay(
                            &desktop.windows,
                            bounds,
                            primary,
                            settings,
                        )
                })
                .map(|primary| primary.id);
            let clear_area = clear
                .iter()
                .map(|rect| i64::from(rect.w) * i64::from(rect.h))
                .sum::<i64>();
            (clear_area < i64::from(bounds.w) * i64::from(bounds.h)).then_some((
                display.id,
                Mask {
                    bounds,
                    clear: coalesce(clear),
                    ambient,
                    #[cfg(target_os = "macos")]
                    behind,
                },
            ))
        })
        .collect()
}

#[cfg(target_os = "macos")]
fn front_window_can_cover_overlay(
    windows: &[Window],
    bounds: Rect,
    primary: &Window,
    settings: &Settings,
) -> bool {
    let mut found_primary = false;
    for window in windows
        .iter()
        .filter(|window| window.bounds.intersection(bounds).is_some())
    {
        if window.id == primary.id {
            found_primary = true;
            continue;
        }
        // Above the anchor, only the focused app's floating windows should
        // stay sharp. Ignored windows below it need their own cutouts.
        if settings.is_ignored(&window.app)
            || !(found_primary && window.focusable
                || !window.focusable && window.app == primary.app)
        {
            return false;
        }
    }
    found_primary
}

fn uncovered(rect: Rect, covered: &[Rect]) -> Vec<Rect> {
    let mut visible = vec![rect];
    let mut remaining = Vec::new();
    for occluder in covered {
        remaining.clear();
        for part in visible.drain(..) {
            part.subtract(*occluder, &mut remaining);
        }
        std::mem::swap(&mut visible, &mut remaining);
        if visible.is_empty() {
            break;
        }
    }
    visible
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{AppInfo, Group};

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_does_not_treat_a_floating_preview_as_the_main_window() {
        let preview = Window {
            id: 1,
            app: "editor".into(),
            bounds: Rect::new(1_510, 94, 166, 133),
            focusable: true,
            corner_radius: 0,
        };
        let main = Window {
            id: 2,
            app: "editor".into(),
            bounds: Rect::new(317, 149, 1_350, 874),
            focusable: true,
            corner_radius: 0,
        };
        assert_eq!(
            mac_foreground(&[preview.clone(), main], "editor")
                .unwrap()
                .id,
            2
        );
        assert_eq!(mac_foreground(&[preview], "editor").unwrap().id, 1);
    }

    #[test]
    fn cutout_rounds_all_four_corners_but_leaves_the_straight_edges_clear() {
        let mut desktop = desktop();
        desktop.windows[0].corner_radius = 12;
        let bounds = desktop.windows[0].bounds;
        let masks = plan(&desktop, &Settings::default());
        let clear = |x, y| masks[0].1.clear.iter().any(|rect| rect.contains(x, y));
        assert!(!clear(bounds.x, bounds.y));
        assert!(!clear(bounds.right() - 1, bounds.y));
        assert!(!clear(bounds.x, bounds.bottom() - 1));
        assert!(!clear(bounds.right() - 1, bounds.bottom() - 1));
        assert!(clear(bounds.x + bounds.w / 2, bounds.y));
        assert!(clear(bounds.x, bounds.y + bounds.h / 2));

        desktop.windows[0].bounds = desktop.displays[0].bounds;
        let masks = plan(&desktop, &Settings::default());
        assert!(!masks.iter().any(|(display, _)| *display == 1));
    }

    #[test]
    fn focus_switch_fades_changed_regions_and_keeps_the_current_alpha_on_retarget() {
        let bounds = Rect::new(0, 0, 100, 80);
        let mask = |x| Mask {
            bounds,
            clear: vec![Rect::new(x, 20, 30, 40)],
            ambient: Vec::new(),
            #[cfg(target_os = "macos")]
            behind: None,
        };
        let start = Instant::now();
        let duration = Duration::from_millis(240);
        let mut coverage = Coverage::new(mask(10), start);
        let alpha = |coverage: &Coverage, now, x, y| {
            let mut tiles = coverage
                .tiles
                .iter()
                .filter(|tile| tile.rect.contains(x, y));
            let opacity = tiles
                .next()
                .map_or(0.0, |tile| tile.opacity(coverage.progress(now)));
            assert!(tiles.next().is_none(), "coverage tiles must not overlap");
            opacity
        };
        coverage.update(&mask(30), start, duration);
        let halfway = start + duration / 2;
        assert_eq!(alpha(&coverage, halfway, 15, 35), 0.5);
        assert_eq!(alpha(&coverage, halfway, 50, 35), 0.5);
        assert_eq!(alpha(&coverage, halfway, 35, 35), 0.0);
        assert_eq!(alpha(&coverage, halfway, 80, 35), 1.0);
        coverage.update(&mask(10), halfway, duration);
        assert_eq!(alpha(&coverage, halfway, 15, 35), 0.5);
        assert_eq!(alpha(&coverage, halfway, 50, 35), 0.5);
        let done = halfway + duration;
        coverage.update(&mask(10), done, duration);
        assert_eq!(alpha(&coverage, done, 15, 35), 0.0);
        assert_eq!(alpha(&coverage, done, 50, 35), 1.0);
        assert!(!coverage.tiles.iter().any(|tile| tile.is_animating()));
    }

    fn desktop() -> Desktop {
        Desktop {
            displays: vec![
                Display {
                    id: 1,
                    bounds: Rect::new(0, 0, 100, 100),
                    work: Rect::new(0, 0, 100, 100),
                    stage: None,
                },
                Display {
                    id: 2,
                    bounds: Rect::new(-100, 0, 100, 100),
                    work: Rect::new(-100, 0, 100, 100),
                    stage: None,
                },
            ],
            windows: vec![
                Window {
                    id: 1,
                    app: "one".into(),
                    bounds: Rect::new(25, 25, 50, 50),
                    focusable: true,
                    corner_radius: 0,
                },
                Window {
                    id: 2,
                    app: "two".into(),
                    bounds: Rect::new(40, 0, 50, 90),
                    focusable: true,
                    corner_radius: 0,
                },
                Window {
                    id: 3,
                    app: "one".into(),
                    bounds: Rect::new(10, 10, 65, 70),
                    focusable: true,
                    corner_radius: 0,
                },
                Window {
                    id: 4,
                    app: "three".into(),
                    bounds: Rect::new(-80, 10, 65, 70),
                    focusable: true,
                    corner_radius: 0,
                },
            ],
            foreground: Some(1),
            foreground_app: Some("one".into()),
            ..Desktop::default()
        }
    }

    #[test]
    fn overlay_bounds_account_for_system_bars() {
        let mut desktop = desktop();
        desktop.displays[0].work = Rect::new(0, 10, 100, 80);
        let display = &desktop.displays[0];
        let mut settings = Settings::default();

        let masks = plan(&desktop, &settings);
        let mask = &masks[0].1;
        if cfg!(target_os = "macos") {
            assert_eq!(mask.bounds, display.bounds);
            assert!(!mask.clear.iter().any(|rect| rect.contains(50, 5)));
            assert!(mask.clear.iter().any(|rect| rect.contains(50, 50)));
        } else {
            assert_eq!(mask.bounds, display.work);
        }

        settings.dim_chrome = true;
        assert_eq!(plan(&desktop, &settings)[0].1.bounds, display.bounds);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn backdrop_can_stay_behind_a_moving_front_window() {
        let mut desktop = desktop();
        let mut settings = Settings::default();
        let first = plan(&desktop, &settings);
        assert_eq!(first[0].1.behind, Some(1));

        desktop.windows[0].bounds.x += 12;
        let moved = plan(&desktop, &settings);
        assert_ne!(moved[0].1.clear, first[0].1.clear);
        assert_eq!(moved[0].1.behind, Some(1));

        settings.ignored.push(AppInfo {
            id: "two".into(),
            name: "Two".into(),
        });
        assert_eq!(plan(&desktop, &settings)[0].1.behind, None);
        settings.ignored.clear();

        desktop.windows.swap(0, 1);
        assert_eq!(plan(&desktop, &settings)[0].1.behind, None);
    }

    #[test]
    fn overview_suppresses_masks_until_the_desktop_returns() {
        let mut desktop = desktop();
        let settings = Settings::default();
        let original = plan(&desktop, &settings);
        assert!(!original.is_empty());
        desktop.overview = true;
        assert!(plan(&desktop, &settings).is_empty());
        desktop.overview = false;
        assert_eq!(plan(&desktop, &settings), original);
    }

    #[test]
    fn app_wide_focus_respects_occlusion_and_display_modes() {
        let desktop = desktop();
        let mut settings = Settings {
            app_wide: true,
            ..Settings::default()
        };
        let masks = plan(&desktop, &settings);
        let clear = &masks[0].1.clear;
        assert!(clear.iter().any(|r| r.contains(12, 12)));
        assert!(clear.iter().any(|r| r.contains(50, 50)));
        assert!(!clear.iter().any(|r| r.contains(50, 20)));
        assert_eq!(masks[1].1.clear, vec![desktop.windows[3].bounds]);
        settings.displays = Displays::Global;
        assert!(plan(&desktop, &settings)[1].1.clear.is_empty());
    }

    #[test]
    fn app_groups_cross_displays_and_ignored_foreground_pauses() {
        let desktop = desktop();
        let app = |id: &str| AppInfo {
            id: id.into(),
            name: id.into(),
        };
        let mut settings = Settings {
            displays: Displays::Global,
            groups: vec![Group {
                name: "Work".into(),
                apps: vec![app("one"), app("three")],
            }],
            ..Settings::default()
        };
        let masks = plan(&desktop, &settings);
        assert_eq!(masks[1].1.clear, vec![desktop.windows[3].bounds]);
        settings.ignored.push(app("one"));
        assert!(plan(&desktop, &settings).is_empty());
    }
}
