use anyhow::{bail, Result};
use serde::Deserialize;

pub const COLS: usize = 6;
pub const CARD_W: f32 = 250.0;
pub const CARD_H: f32 = 150.0;
pub const FOCUSED_SCALE: f32 = 1.14;
pub const STEP_X: f32 = 294.0;
pub const STEP_Y: f32 = 242.0;
pub const LEFT: f32 = 100.0;
pub const PAD: f32 = 35.2;
pub const HEADER_TOP: f32 = 62.0;
pub const HEADER_ICON_SIZE: f32 = 32.0;
pub const HEADER_ICON_GAP: f32 = 40.0;
pub const HEADER_RIGHT: f32 = LEFT + STEP_X * (COLS - 1) as f32 + CARD_W + PAD;
pub const HEADER_PANEL_TOP: f32 = HEADER_TOP + 36.0 + HEADER_ICON_SIZE / 2.0;
pub const SETTINGS_PANEL_LEFT: f32 = HEADER_RIGHT - HEADER_ICON_SIZE - HEADER_ICON_GAP - 600.0;
pub const WIFI_PANEL_LEFT: f32 = HEADER_RIGHT - 680.0;
pub const SPEED_PANEL_LEFT: f32 = HEADER_RIGHT - 900.0;
pub const PANEL_ROW_TOP: f32 = HEADER_PANEL_TOP + 100.0;
pub const PANEL_ROW_STEP: f32 = 88.0;
pub const PIN_LEFT: f32 = SETTINGS_PANEL_LEFT + 500.0;
pub fn weather_bounds(count: usize) -> (f32, f32, f32, f32) {
    if count > COLS * 2 {
        (LEFT, HEADER_TOP, 660.0, 152.0)
    } else {
        (LEFT, HEADER_TOP, 660.0, 250.0)
    }
}
pub fn weather_action_bounds(index: usize) -> (f32, f32, f32, f32) {
    (LEFT + 24.0, 632.0 + index as f32 * 88.0, 800.0, 76.0)
}
pub fn weather_focus_bounds(count: usize) -> (f32, f32, f32, f32) {
    let (x, y, w, h) = weather_bounds(count);
    (x - 28.0, y + 4.0, w + 56.0, h + 28.0)
}

pub fn tile_focus(scale: f32) -> f32 {
    ((scale - 1.0) / (FOCUSED_SCALE - 1.0)).clamp(0.0, 1.0)
}

/// Rendering and pointer selection share the tile's animated bounds.
pub fn tile_bounds(position: (f32, f32), scroll: f32, scale: f32) -> (f32, f32, f32, f32) {
    let (w, h) = (CARD_W * scale, CARD_H * scale);
    (
        position.0 - (w - CARD_W) / 2.0,
        position.1 - scroll - (h - CARD_H) / 2.0,
        w,
        h,
    )
}
/// Wi-Fi is the rightmost control, aligned with the tray's right edge.
pub fn wifi_bounds() -> (f32, f32, f32, f32) {
    (
        HEADER_RIGHT - 36.0 - HEADER_ICON_SIZE / 2.0,
        HEADER_TOP,
        72.0,
        72.0,
    )
}

pub fn wifi_action_bounds(index: usize) -> (f32, f32, f32, f32) {
    (
        WIFI_PANEL_LEFT + 24.0,
        HEADER_PANEL_TOP + 460.0 + index as f32 * 88.0,
        632.0,
        76.0,
    )
}
pub fn speed_action_bounds(index: usize) -> (f32, f32, f32, f32) {
    (
        SPEED_PANEL_LEFT + 24.0,
        HEADER_PANEL_TOP + 492.0 + index as f32 * 88.0,
        852.0,
        76.0,
    )
}

/// Settings sits between the clock and Wi-Fi.
pub fn status_bounds() -> (f32, f32, f32, f32) {
    (
        wifi_bounds().0 - HEADER_ICON_SIZE - HEADER_ICON_GAP,
        HEADER_TOP,
        72.0,
        72.0,
    )
}

#[derive(Deserialize, Clone)]
pub struct App {
    pub id: String,
    pub title: String,
    pub artwork: String,
    pub contain: bool,
    pub color: [u8; 3],
    #[serde(default)]
    pub background: Option<[u8; 3]>,
    #[serde(skip)]
    pub input: bool,
}

#[derive(Deserialize, Clone)]
pub struct Input {
    #[serde(rename = "appId")]
    pub app_id: String,
    pub label: String,
}

#[derive(Deserialize, Clone)]
pub struct Catalog {
    pub preview: bool,
    pub apps: Vec<App>,
    pub inputs: Vec<Input>,
}

impl Catalog {
    pub fn input_items(&self) -> Vec<Input> {
        let mut inputs = self.inputs.clone();
        if !inputs.iter().any(|i| i.app_id == "com.webos.app.livetv") {
            inputs.push(Input {
                app_id: "com.webos.app.livetv".into(),
                label: "Live TV".into(),
            });
        }
        inputs
    }

    pub fn validate(&self, preview: bool) -> Result<()> {
        if self.apps.is_empty() {
            bail!("Home's app list is empty. Install a package prepared for this TV.");
        }
        if !preview && (self.preview || self.apps.iter().any(|a| a.id.starts_with("preview."))) {
            bail!("This package contains preview data. Install a TV package.");
        }
        if self.apps.len() > 512 || self.inputs.len() > 64 {
            bail!("Catalog contains too many entries.");
        }
        let mut ids = std::collections::HashSet::new();
        for app in &self.apps {
            if !ids.insert(&app.id) {
                bail!("Catalog contains duplicate app IDs.");
            }
            if std::path::Path::new(&app.artwork)
                .components()
                .any(|c| !matches!(c, std::path::Component::Normal(_)))
            {
                bail!("Catalog contains an unsafe artwork path.");
            }
        }
        if self.apps.iter().any(|a| a.id.is_empty()) {
            bail!("Catalog contains an empty app ID.");
        }
        Ok(())
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Direction {
    Left,
    Right,
    Up,
    Down,
}

/// None moves focus from the top row to the header controls.
pub fn next(index: usize, direction: Direction, count: usize) -> Option<usize> {
    if count == 0 {
        return None;
    }
    let index = index.min(count - 1);
    Some(match direction {
        Direction::Left if !index.is_multiple_of(COLS) => index - 1,
        Direction::Right if index % COLS != COLS - 1 => (index + 1).min(count - 1),
        Direction::Up if index < COLS => return None,
        Direction::Up => index - COLS,
        Direction::Down if index / COLS < (count - 1) / COLS => (index + COLS).min(count - 1),
        _ => index,
    })
}

pub fn grid_top(count: usize) -> f32 {
    let rows = count.div_ceil(COLS).clamp(1, 3);
    let tail = if rows == 1 { 100.0 } else { 70.0 };
    1025.0 - CARD_H - tail - (rows - 1) as f32 * STEP_Y - if rows > 1 { 32.0 } else { 0.0 }
}

pub fn position(index: usize, count: usize) -> (f32, f32) {
    let row = index / COLS;
    (
        LEFT + (index % COLS) as f32 * STEP_X,
        grid_top(count) + row as f32 * STEP_Y + if row > 0 { 32.0 } else { 0.0 },
    )
}

pub fn scroll_for(index: usize, count: usize, current: f32) -> f32 {
    if index < COLS {
        return 0.0;
    }
    // A catalog can shrink while focus is below the first row. Drop obsolete scroll space.
    let maximum = (position(count.saturating_sub(1), count).1 + CARD_H + 70.0 - 1025.0).max(0.0);
    let current = current.clamp(0.0, maximum);
    let y = position(index, count).1;
    let top = if index < COLS {
        y - PAD - 12.0
    } else {
        y - 16.0
    };
    let bottom = y + CARD_H + if index < COLS { 100.0 } else { 70.0 };
    if top - current < 190.0 {
        (top - 190.0).max(0.0)
    } else if bottom - current > 1025.0 {
        bottom - 1025.0
    } else {
        current
    }
}

/// A tile keeps its current position when a new destination interrupts a slide.
#[derive(Clone, Copy)]
pub struct TileMotion {
    pub from: (f32, f32),
    pub to: (f32, f32),
    pub start: std::time::Instant,
}
impl TileMotion {
    const DURATION: std::time::Duration = std::time::Duration::from_millis(180);

    pub fn new(position: (f32, f32)) -> Self {
        Self {
            from: position,
            to: position,
            start: std::time::Instant::now(),
        }
    }
    pub fn value_at(&self, now: std::time::Instant) -> (f32, f32) {
        let t = (now.saturating_duration_since(self.start).as_secs_f32()
            / Self::DURATION.as_secs_f32())
        .min(1.0);
        if t >= 1.0 {
            return self.to;
        }
        let eased = 1.0 - (1.0 - t).powi(3);
        (
            self.from.0 + (self.to.0 - self.from.0) * eased,
            self.from.1 + (self.to.1 - self.from.1) * eased,
        )
    }
    pub fn retarget(&mut self, position: (f32, f32), now: std::time::Instant) {
        if self.to != position {
            self.from = self.value_at(now);
            self.to = position;
            self.start = now;
        }
    }
    pub fn active(&self) -> bool {
        self.from != self.to && self.start.elapsed() < Self::DURATION
    }
}

#[derive(Clone, Copy)]
pub struct Tween {
    pub from: f32,
    pub to: f32,
    pub start: std::time::Instant,
}

impl Tween {
    pub fn new(value: f32) -> Self {
        Self {
            from: value,
            to: value,
            start: std::time::Instant::now(),
        }
    }
    pub fn value(&self) -> f32 {
        let t = (self.start.elapsed().as_secs_f32() / 0.10).min(1.0);
        self.from + (self.to - self.from) * (1.0 - (1.0 - t).powi(3))
    }
    pub fn set(&mut self, value: f32) {
        if self.to != value {
            self.from = self.value();
            self.to = value;
            self.start = std::time::Instant::now();
        }
    }
    pub fn active(&self) -> bool {
        self.from != self.to && self.start.elapsed().as_millis() < 100
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tile_slide_retargets_without_jumping_or_restarting_unchanged_destinations() {
        use std::time::{Duration, Instant};
        let now = Instant::now();
        let mut tile = TileMotion::new((100.0, 300.0));
        tile.retarget((394.0, 574.0), now);
        assert_eq!(tile.value_at(now), (100.0, 300.0));
        let halfway = now + Duration::from_millis(90);
        let current = tile.value_at(halfway);
        assert!(current.0 > 100.0 && current.0 < 394.0);
        assert!(current.1 > 300.0 && current.1 < 574.0);
        tile.retarget(tile.to, halfway);
        assert_eq!(tile.start, now);
        assert_eq!(tile.value_at(halfway), current);
        tile.retarget((688.0, 300.0), halfway);
        assert_eq!(tile.value_at(halfway), current);
        assert_eq!(
            tile.value_at(halfway + Duration::from_millis(180)),
            (688.0, 300.0)
        );
    }
    #[test]
    fn ragged_rows_and_edges() {
        assert_eq!(next(5, Direction::Down, 8), Some(7));
        assert_eq!(next(7, Direction::Right, 8), Some(7));
        assert_eq!(next(6, Direction::Left, 8), Some(6));
        assert_eq!(next(7, Direction::Up, 8), Some(1));
        assert_eq!(next(2, Direction::Up, 8), None);
        assert_eq!(next(0, Direction::Down, 0), None);
    }
    #[test]
    fn short_grids_are_bottom_aligned() {
        assert_eq!(position(0, 6).1 + CARD_H + 100.0, 1025.0);
        assert_eq!(position(6, 8).1 + CARD_H + 70.0, 1025.0);
        assert_eq!(position(12, 18).1 + CARD_H + 70.0, 1025.0);
        assert!(grid_top(18) - PAD > 190.0);
    }
    #[test]
    fn scrolling_reveals_focus_and_returns_to_tray() {
        let scroll = scroll_for(24, 25, 0.0);
        assert!(scroll > 0.0);
        assert_eq!(position(24, 25).1 - scroll + CARD_H + 70.0, 1025.0);
        assert_eq!(scroll_for(0, 25, scroll), 0.0);
        assert_eq!(scroll_for(7, 8, 0.0), 0.0);
        assert_eq!(scroll_for(9, 10, scroll), 0.0);
        assert_eq!(scroll_for(17, 18, scroll), 0.0);
    }
    #[test]
    fn reject_preview_catalog_on_tv() {
        let c = Catalog {
            preview: true,
            apps: vec![App {
                id: "preview.netflix".into(),
                title: "Netflix".into(),
                artwork: String::new(),
                contain: false,
                color: [0, 0, 0],
                background: None,
                input: false,
            }],
            inputs: vec![],
        };
        assert!(c.validate(false).is_err());
        assert!(c.validate(true).is_ok());
    }
}
