use anyhow::{bail, Result};
use serde::Deserialize;

pub const COLS: usize = 6;
pub const CARD_W: f32 = 250.0;
pub const CARD_H: f32 = 150.0;
pub const STEP_X: f32 = 294.0;
pub const STEP_Y: f32 = 242.0;
pub const LEFT: f32 = 100.0;
pub const PAD: f32 = 35.2;
pub const PANEL_ROW_TOP: f32 = 260.0;
pub const PANEL_ROW_STEP: f32 = 88.0;
/// The Wi-Fi target mirrors the clock height and aligns with the tray's left edge.
pub fn wifi_bounds() -> (f32, f32, f32, f32) {
    (LEFT - PAD, 62.0, 72.0, 72.0)
}

pub fn wifi_close_bounds() -> (f32, f32, f32, f32) {
    (LEFT - PAD + 24.0, 620.0, 632.0, 76.0)
}

/// Left, top, width and height; the status pill shares the tray's right edge.
pub fn status_bounds() -> (f32, f32, f32, f32) {
    let width = 246.0;
    let tray_right = LEFT + STEP_X * (COLS - 1) as f32 + CARD_W + PAD;
    (tray_right - width, 62.0, width, 72.0)
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
}

#[derive(Deserialize, Clone)]
pub struct Input {
    #[serde(rename = "appId")]
    pub app_id: String,
    pub label: String,
}

#[derive(Deserialize)]
pub struct Catalog {
    pub preview: bool,
    pub apps: Vec<App>,
    pub inputs: Vec<Input>,
}

impl Catalog {
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
            }],
            inputs: vec![],
        };
        assert!(c.validate(false).is_err());
        assert!(c.validate(true).is_ok());
    }
}
