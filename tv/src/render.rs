use crate::model::*;
use crate::platform::wifi::{Connection, Status};
use anyhow::{Context, Result};
use skia_safe::{
    AlphaType, Canvas, Color, ColorType, Data, Font, FontMgr, Image, ImageInfo, Paint, Rect,
};
use std::path::Path;
use std::sync::mpsc;

pub struct Frame<'a> {
    pub catalog: &'a Catalog,
    pub selected: usize,
    pub status: bool,
    pub scroll: f32,
    pub scales: &'a [f32],
    pub positions: &'a [(f32, f32)],
    pub clock: &'a str,
    pub date: &'a str,
    pub panel: Option<(&'a [String], usize)>,
    pub input_pins: Option<&'a [bool]>,
    pub pin_focused: bool,
    pub wifi: &'a Status,
    pub wifi_focused: bool,
    pub wifi_details: bool,
    pub wifi_selection: usize,
    pub speed_test: Option<(&'a crate::speedtest::State, usize)>,
    pub move_elapsed: Option<f32>,
    pub move_hint: Option<&'a str>,
    pub toast: Option<&'a str>,
    pub metrics: Option<&'a str>,
}

pub struct Renderer {
    font: Font,
    pub icons: Vec<Option<Image>>,
    glow: Image,
}

fn paint(color: Color) -> Paint {
    let mut p = Paint::default();
    p.set_color(color).set_anti_alias(true);
    p
}
fn rgba_image(bytes: &[u8], w: u32, h: u32) -> Result<Image> {
    let info = ImageInfo::new(
        (w as i32, h as i32),
        ColorType::RGBA8888,
        AlphaType::Unpremul,
        None,
    );
    skia_safe::images::raster_from_data(&info, Data::new_copy(bytes), w as usize * 4)
        .context("Cannot create image")
}

/// Decode, downsize and round each icon once, away from the rendering thread.
pub fn decode_icons(root: &Path, apps: &[App]) -> mpsc::Receiver<(usize, Result<Image>)> {
    let (tx, rx) = mpsc::channel();
    let root = root.to_owned();
    let apps = apps.to_vec();
    std::thread::spawn(move || {
        for (i, app) in apps.iter().enumerate() {
            if app.artwork.is_empty() {
                continue;
            }
            let result = (|| {
                let source = image::ImageReader::open(root.join(&app.artwork))?
                    .with_guessed_format()?
                    .decode()?;
                let (w, h) = (300u32, 180u32);
                let scaled = if app.contain {
                    source.resize(148, 148, image::imageops::FilterType::Triangle)
                } else {
                    source.resize_to_fill(w, h, image::imageops::FilterType::Triangle)
                }
                .to_rgba8();
                let [r, g, b] = app.background.unwrap_or([23, 24, 34]);
                let mut pixels = image::RgbaImage::from_pixel(w, h, image::Rgba([r, g, b, 255]));
                image::imageops::overlay(
                    &mut pixels,
                    &scaled,
                    ((w - scaled.width()) / 2) as i64,
                    ((h - scaled.height()) / 2) as i64,
                );
                // Rounded alpha is baked at texture resolution, not clipped on every GPU frame.
                for y in 0..h {
                    for x in 0..w {
                        let radius = 17.0f32;
                        let dx = (radius - x as f32 - 0.5)
                            .max(x as f32 + 0.5 - (w as f32 - radius))
                            .max(0.0);
                        let dy = (radius - y as f32 - 0.5)
                            .max(y as f32 + 0.5 - (h as f32 - radius))
                            .max(0.0);
                        let alpha = (radius + 0.5 - (dx * dx + dy * dy).sqrt()).clamp(0.0, 1.0);
                        pixels.get_pixel_mut(x, y)[3] = (alpha * 255.0) as u8;
                    }
                }
                rgba_image(pixels.as_raw(), w, h)
            })();
            if tx.send((i, result)).is_err() {
                break;
            }
        }
    });
    rx
}

impl Renderer {
    pub fn new(root: &Path, count: usize) -> Result<Self> {
        let bytes = std::fs::read(root.join("Inter.ttf"))?;
        let typeface = FontMgr::new()
            .new_from_data(Data::new_copy(&bytes), None)
            .context("Cannot load Inter")?;
        let mut font = Font::new(typeface, 26.0);
        font.set_subpixel(true);
        // A small cached texture: no runtime blur shader or full-screen offscreen pass.
        let mut pixels = vec![0u8; 256 * 96 * 4];
        for y in 0..96 {
            for x in 0..256 {
                let dx = (x as f32 - 127.5) / 100.0;
                let dy = (y as f32 - 47.5) / 31.0;
                let a = ((-(dx * dx + dy * dy) * 2.0).exp() * 255.0) as u8;
                let offset = (y * 256 + x) * 4;
                pixels[offset..offset + 4].copy_from_slice(&[255, 255, 255, a]);
            }
        }
        Ok(Self {
            font,
            icons: vec![None; count],
            glow: rgba_image(&pixels, 256, 96)?,
        })
    }

    // Explicit size/alignment keeps the fixed 1080p layout easy to audit.
    #[allow(clippy::too_many_arguments)]
    fn text(
        &self,
        canvas: &Canvas,
        text: &str,
        x: f32,
        y: f32,
        size: f32,
        color: Color,
        centered: bool,
    ) {
        let mut font = self.font.clone();
        font.set_size(size);
        let width = if centered {
            font.measure_str(text, None).0 / 2.0
        } else {
            0.0
        };
        canvas.draw_str(text, (x - width, y), &font, &paint(color));
    }

    pub fn draw(&self, canvas: &Canvas, f: &Frame<'_>, size: (u32, u32)) {
        canvas.clear(Color::BLACK);
        canvas.save();
        canvas.scale((size.0 as f32 / 1920.0, size.1 as f32 / 1080.0));
        let top = grid_top(f.catalog.apps.len()) - f.scroll;
        let [r, g, b] = f.catalog.apps[f.selected].color;
        let mut p = paint(Color::WHITE);
        p.set_color_filter(skia_safe::color_filters::blend(
            Color::from_argb(120, r, g, b),
            skia_safe::BlendMode::SrcIn,
        ));
        canvas.draw_image_rect_with_sampling_options(
            &self.glow,
            None,
            Rect::from_xywh(LEFT - 145.0, top - 145.0, 1808.0 + 200.0, 510.0),
            skia_safe::FilterMode::Linear,
            &p,
        );
        // Solid translucent-looking tray. No expensive backdrop readback or blur.
        let tray = Rect::from_xywh(
            LEFT - PAD,
            top - PAD,
            STEP_X * (COLS - 1) as f32 + CARD_W + PAD * 2.0,
            CARD_H + PAD * 2.0,
        );
        canvas.draw_round_rect(tray, 34.0, 34.0, &paint(Color::from_argb(225, 35, 36, 43)));
        let mut stroke = paint(Color::from_argb(50, 255, 255, 255));
        stroke
            .set_style(skia_safe::paint::Style::Stroke)
            .set_stroke_width(1.2);
        canvas.draw_round_rect(tray, 34.0, 34.0, &stroke);
        for i in 0..f.catalog.apps.len() {
            if i != f.selected {
                self.card(canvas, f, i);
            }
        }
        self.card(canvas, f, f.selected);

        self.wifi_icon(canvas, f.wifi, f.wifi_focused);

        // The status button stays separate from scrolling content.
        let (status_x, status_y, status_w, status_h) = status_bounds();
        canvas.draw_round_rect(
            Rect::from_xywh(status_x, status_y, status_w, status_h),
            36.0,
            36.0,
            &paint(if f.status {
                Color::from_rgb(220, 221, 227)
            } else {
                Color::from_argb(225, 34, 35, 41)
            }),
        );
        self.text(
            canvas,
            f.clock,
            status_x + 90.0,
            status_y + 46.0,
            27.0,
            if f.status { Color::BLACK } else { Color::WHITE },
            true,
        );
        let symbol = if f.status {
            Color::BLACK
        } else {
            Color::from_rgb(210, 211, 218)
        };
        // Eight rounded teeth and an open centre, like Apple's gearshape symbol.
        canvas.save();
        canvas.translate((status_x + 191.0, status_y + 36.0));
        let mut gear = paint(symbol);
        for _ in 0..8 {
            canvas.draw_round_rect(Rect::from_xywh(-2.6, -16.0, 5.2, 8.0), 1.2, 1.2, &gear);
            canvas.rotate(45.0, None);
        }
        gear.set_style(skia_safe::paint::Style::Stroke)
            .set_stroke_width(5.0);
        canvas.draw_circle((0.0, 0.0), 9.5, &gear);
        canvas.restore();
        if let Some((items, selected)) = f.panel {
            canvas.draw_rect(
                Rect::from_wh(1920.0, 1080.0),
                &paint(Color::from_argb(160, 0, 0, 0)),
            );
            let h = PANEL_ROW_TOP - 160.0 + items.len() as f32 * PANEL_ROW_STEP + 12.0;
            canvas.draw_round_rect(
                Rect::from_xywh(1220.0, 160.0, 600.0, h),
                36.0,
                36.0,
                &paint(Color::from_rgb(32, 33, 40)),
            );
            self.text(
                canvas,
                f.date,
                1272.0,
                216.0,
                22.0,
                Color::from_rgb(164, 165, 177),
                false,
            );
            canvas.draw_rect(
                Rect::from_xywh(1272.0, 238.0, 496.0, 1.0),
                &paint(Color::from_argb(32, 255, 255, 255)),
            );
            for (i, item) in items.iter().enumerate() {
                let y = PANEL_ROW_TOP + 24.0 + i as f32 * PANEL_ROW_STEP;
                let label_focused = i == selected && !(f.input_pins.is_some() && f.pin_focused);
                if label_focused {
                    canvas.draw_round_rect(
                        Rect::from_xywh(
                            1244.0,
                            y - 24.0,
                            if f.input_pins.is_some() {
                                PIN_LEFT - 12.0 - 1244.0
                            } else {
                                552.0
                            },
                            76.0,
                        ),
                        18.0,
                        18.0,
                        &paint(Color::from_rgb(223, 224, 230)),
                    );
                }
                let label = self.ellipsized(
                    item,
                    if f.input_pins.is_some() { 420.0 } else { 496.0 },
                    28.0,
                );
                self.text(
                    canvas,
                    &label,
                    1272.0,
                    y + 24.0,
                    28.0,
                    if label_focused {
                        Color::BLACK
                    } else {
                        Color::WHITE
                    },
                    false,
                );
                if let Some(pins) = f.input_pins {
                    let active = i == selected && f.pin_focused;
                    if active {
                        canvas.draw_round_rect(
                            Rect::from_xywh(PIN_LEFT, y - 24.0, 76.0, 76.0),
                            18.0,
                            18.0,
                            &paint(Color::from_rgb(223, 224, 230)),
                        );
                    }
                    self.pin(
                        canvas,
                        PIN_LEFT + 38.0,
                        y + 14.0,
                        pins.get(i).copied().unwrap_or(false),
                        if active {
                            Color::BLACK
                        } else {
                            Color::from_rgb(198, 201, 214)
                        },
                    );
                }
            }
        }
        if f.wifi_details {
            let left = LEFT - PAD;
            let text_x = left + 40.0;
            let muted = Color::from_rgb(164, 165, 177);
            canvas.draw_rect(
                Rect::from_wh(1920.0, 1080.0),
                &paint(Color::from_argb(160, 0, 0, 0)),
            );
            canvas.draw_round_rect(
                Rect::from_xywh(left, 160.0, 680.0, 648.0),
                36.0,
                36.0,
                &paint(Color::from_rgb(32, 33, 40)),
            );
            self.text(canvas, "Wi-Fi", text_x, 214.0, 30.0, Color::WHITE, false);
            self.wrapped_text(
                canvas,
                f.wifi.name.as_deref().unwrap_or("Network unavailable"),
                Rect::from_xywh(text_x, 240.0, 600.0, 60.0),
                Color::WHITE,
            );
            self.text(
                canvas,
                f.wifi.description(),
                text_x,
                328.0,
                22.0,
                muted,
                false,
            );
            if f.wifi.preview {
                self.text(
                    canvas,
                    "Preview data",
                    left + 480.0,
                    214.0,
                    20.0,
                    muted,
                    false,
                );
            }
            canvas.draw_rect(
                Rect::from_xywh(text_x, 352.0, 600.0, 1.0),
                &paint(Color::from_argb(32, 255, 255, 255)),
            );
            for (i, (label, value)) in [
                ("Signal", f.wifi.signal_label()),
                (
                    "IP address",
                    f.wifi.ip.clone().unwrap_or_else(|| "Unavailable".into()),
                ),
                (
                    "Router",
                    f.wifi
                        .gateway
                        .clone()
                        .unwrap_or_else(|| "Unavailable".into()),
                ),
            ]
            .iter()
            .enumerate()
            {
                let y = 386.0 + i as f32 * 80.0;
                self.text(canvas, label, text_x, y, 20.0, muted, false);
                self.wrapped_text(
                    canvas,
                    value,
                    Rect::from_xywh(text_x, y + 12.0, 600.0, 30.0),
                    Color::WHITE,
                );
            }
            for (i, label) in ["Speed test", "Close"].iter().enumerate() {
                let (x, y, _, _) = wifi_action_bounds(i);
                let focused = i == f.wifi_selection;
                self.text(
                    canvas,
                    label,
                    x + 16.0,
                    y + 48.0,
                    if focused { 30.0 } else { 28.0 },
                    if focused { Color::WHITE } else { muted },
                    false,
                );
            }
        }
        if let Some((state, selected)) = f.speed_test {
            self.speed_test_panel(canvas, state, selected);
        }
        if let Some(toast) = f.toast {
            canvas.draw_round_rect(
                Rect::from_xywh(340.0, 172.0, 1240.0, 74.0),
                25.0,
                25.0,
                &paint(Color::from_rgb(44, 45, 52)),
            );
            // Bound service error text to one line.
            let short: String = toast.chars().take(80).collect();
            self.text(canvas, &short, 960.0, 219.0, 25.0, Color::WHITE, true);
        } else if let Some(hint) = f.move_hint {
            self.text(
                canvas,
                hint,
                960.0,
                197.0,
                24.0,
                Color::from_rgb(205, 206, 215),
                true,
            );
        }
        if let Some(metrics) = f.metrics {
            self.text(
                canvas,
                metrics,
                64.0,
                1045.0,
                20.0,
                Color::from_rgb(180, 210, 195),
                false,
            );
        }
        canvas.restore();
    }

    fn panel_button(
        &self,
        canvas: &Canvas,
        label: &str,
        bounds: (f32, f32, f32, f32),
        focused: bool,
    ) {
        let (x, y, w, h) = bounds;
        if focused {
            canvas.draw_round_rect(
                Rect::from_xywh(x, y, w, h),
                18.0,
                18.0,
                &paint(Color::from_rgb(223, 224, 230)),
            );
        }
        self.text(
            canvas,
            label,
            x + 16.0,
            y + 48.0,
            28.0,
            if focused { Color::BLACK } else { Color::WHITE },
            false,
        );
    }

    fn speed_test_panel(&self, canvas: &Canvas, state: &crate::speedtest::State, selected: usize) {
        let left = LEFT - PAD;
        let x = left + 40.0;
        let muted = Color::from_rgb(164, 165, 177);
        canvas.draw_rect(
            Rect::from_wh(1920.0, 1080.0),
            &paint(Color::from_argb(160, 0, 0, 0)),
        );
        canvas.draw_round_rect(
            Rect::from_xywh(left, 160.0, 900.0, 680.0),
            36.0,
            36.0,
            &paint(Color::from_rgb(32, 33, 40)),
        );
        self.text(canvas, "Speed test", x, 214.0, 30.0, Color::WHITE, false);
        self.text(
            canvas,
            "Fast.com · Netflix servers",
            x,
            254.0,
            22.0,
            muted,
            false,
        );
        if state.preview {
            self.text(
                canvas,
                "Emulator connection",
                left + 610.0,
                214.0,
                20.0,
                muted,
                false,
            );
        }
        canvas.draw_rect(
            Rect::from_xywh(x, 286.0, 820.0, 1.0),
            &paint(Color::from_argb(32, 255, 255, 255)),
        );
        for (i, (label, value)) in [("Download", state.download), ("Upload", state.upload)]
            .iter()
            .enumerate()
        {
            let mx = x + i as f32 * 420.0;
            self.text(canvas, label, mx, 334.0, 22.0, muted, false);
            let value = value
                .map(|v| {
                    if v < 10.0 {
                        format!("{v:.1}")
                    } else {
                        format!("{v:.0}")
                    }
                })
                .unwrap_or_else(|| "—".into());
            self.text(canvas, &value, mx, 412.0, 64.0, Color::WHITE, false);
            self.text(canvas, "Mbps", mx, 450.0, 20.0, muted, false);
        }
        self.text(canvas, "Latency", x, 506.0, 22.0, muted, false);
        let latency = state
            .latency
            .map(|v| format!("{v:.0} ms"))
            .unwrap_or_else(|| "—".into());
        self.text(
            canvas,
            &latency,
            x + 130.0,
            506.0,
            26.0,
            Color::WHITE,
            false,
        );
        self.text(
            canvas,
            &self.ellipsized(&state.message, 820.0, 22.0),
            x,
            552.0,
            22.0,
            Color::WHITE,
            false,
        );
        if state.phase.active() {
            canvas.draw_round_rect(
                Rect::from_xywh(x, 576.0, 820.0, 4.0),
                2.0,
                2.0,
                &paint(Color::from_rgb(65, 66, 76)),
            );
            canvas.draw_round_rect(
                Rect::from_xywh(x, 576.0, 820.0 * state.progress.max(0.02), 4.0),
                2.0,
                2.0,
                &paint(Color::from_rgb(185, 202, 238)),
            );
        }
        let note = if let Some(last) = &state.last {
            format!(
                "Last completed: {} · {:.0} down / {:.0} up Mbps",
                last.when, last.download, last.upload
            )
        } else {
            "Runs for about 30 seconds. Uses your internet connection.".into()
        };
        self.text(
            canvas,
            &self.ellipsized(&note, 820.0, 19.0),
            x,
            620.0,
            19.0,
            muted,
            false,
        );
        for (i, label) in [state.button(), "Back"].iter().enumerate() {
            self.panel_button(canvas, label, speed_action_bounds(i), i == selected);
        }
    }

    fn wifi_icon(&self, canvas: &Canvas, status: &Status, focused: bool) {
        let (x, y, w, h) = wifi_bounds();
        let background = if focused {
            Color::from_rgb(220, 221, 227)
        } else {
            Color::BLACK
        };
        if focused {
            canvas.draw_round_rect(Rect::from_xywh(x, y, w, h), 36.0, 36.0, &paint(background));
        }
        let foreground = if focused {
            Color::BLACK
        } else {
            Color::from_rgb(220, 221, 227)
        };
        let inactive = if focused {
            Color::from_rgb(153, 154, 160)
        } else {
            Color::from_rgb(72, 73, 82)
        };
        let cx = x + w / 2.0;
        let cy = y + h / 2.0 + 12.0;
        let connected = status.connection == Connection::Connected;
        let mut stroke = paint(foreground);
        stroke
            .set_style(skia_safe::paint::Style::Stroke)
            .set_stroke_width(4.5)
            .set_stroke_cap(skia_safe::paint::Cap::Round);
        for bar in 1..=3 {
            let radius = 11.0 + (bar - 1) as f32 * 10.0;
            stroke.set_color(
                if connected && status.bars.is_some_and(|bars| bars >= bar) {
                    foreground
                } else {
                    inactive
                },
            );
            canvas.draw_arc(
                Rect::from_xywh(cx - radius, cy - radius, radius * 2.0, radius * 2.0),
                225.0,
                90.0,
                false,
                &stroke,
            );
        }
        canvas.draw_circle(
            (cx, cy),
            3.2,
            &paint(if connected && status.bars.is_some() {
                foreground
            } else {
                inactive
            }),
        );
        if matches!(
            status.connection,
            Connection::Disconnected | Connection::Disabled
        ) {
            stroke.set_color(background).set_stroke_width(9.0);
            canvas.draw_line((cx - 24.0, cy - 30.0), (cx + 20.0, cy + 6.0), &stroke);
            stroke.set_color(foreground).set_stroke_width(3.5);
            canvas.draw_line((cx - 24.0, cy - 30.0), (cx + 20.0, cy + 6.0), &stroke);
        } else if status.bars.is_none() {
            // A missing reading must not look like a weak or disconnected signal.
            let mut font = self.font.clone();
            font.set_size(26.0);
            let text_x = cx - font.measure_str("?", None).0 / 2.0;
            stroke.set_color(background).set_stroke_width(7.0);
            canvas.draw_str("?", (text_x, cy + 2.0), &font, &stroke);
            canvas.draw_str("?", (text_x, cy + 2.0), &font, &paint(foreground));
        }
    }

    /// Keep long network names and addresses inside the details panel.
    fn wrapped_text(&self, canvas: &Canvas, text: &str, bounds: Rect, color: Color) {
        let mut font = self.font.clone();
        font.set_size(22.0);
        let mut lines = Vec::new();
        let mut line = String::new();
        for word in text.split_whitespace() {
            let candidate = if line.is_empty() {
                word.to_owned()
            } else {
                format!("{line} {word}")
            };
            if !line.is_empty() && font.measure_str(&candidate, None).0 > bounds.width() {
                lines.push(std::mem::take(&mut line));
            }
            if !line.is_empty() {
                line.push(' ');
            }
            for ch in word.chars() {
                let candidate = format!("{line}{ch}");
                if !line.is_empty() && font.measure_str(&candidate, None).0 > bounds.width() {
                    lines.push(std::mem::take(&mut line));
                }
                line.push(ch);
            }
        }
        if !line.is_empty() {
            lines.push(line);
        }
        let max_lines = (bounds.height() / 30.0) as usize;
        for (i, line) in lines.iter().take(max_lines).enumerate() {
            let mut visible = line.clone();
            if i + 1 == max_lines && lines.len() > max_lines {
                while !visible.is_empty()
                    && font.measure_str(format!("{visible}…"), None).0 > bounds.width()
                {
                    visible.pop();
                }
                visible.push('…');
            }
            canvas.draw_str(
                &visible,
                (bounds.left(), bounds.top() + 22.0 + i as f32 * 30.0),
                &font,
                &paint(color),
            );
        }
    }

    fn ellipsized(&self, text: &str, width: f32, size: f32) -> String {
        let mut font = self.font.clone();
        font.set_size(size);
        if font.measure_str(text, None).0 <= width {
            return text.into();
        }
        let mut value = text.to_owned();
        while !value.is_empty() && font.measure_str(format!("{value}…"), None).0 > width {
            value.pop();
        }
        value.push('…');
        value
    }

    fn pin(&self, canvas: &Canvas, x: f32, y: f32, filled: bool, color: Color) {
        canvas.save();
        canvas.translate((x, y));
        canvas.scale((0.8, 0.8));
        let mut path = skia_safe::PathBuilder::new();
        path.move_to((-10.0, -17.0))
            .line_to((10.0, -17.0))
            .line_to((7.0, -12.0))
            .line_to((7.0, -1.0))
            .line_to((13.0, 5.0))
            .line_to((-13.0, 5.0))
            .line_to((-7.0, -1.0))
            .line_to((-7.0, -12.0))
            .close();
        let mut p = paint(color);
        p.set_stroke_width(2.5)
            .set_stroke_join(skia_safe::paint::Join::Round);
        if !filled {
            p.set_style(skia_safe::paint::Style::Stroke);
        }
        canvas.draw_path(&path.detach(), &p);
        p.set_style(skia_safe::paint::Style::Stroke)
            .set_stroke_cap(skia_safe::paint::Cap::Round);
        canvas.draw_line((0.0, 5.0), (0.0, 20.0), &p);
        canvas.restore();
    }

    fn card(&self, canvas: &Canvas, f: &Frame<'_>, i: usize) {
        let (x, y) = f.positions[i];
        let y = y - f.scroll;
        if !(-200.0..1100.0).contains(&y) {
            return;
        }
        let scale = f.scales[i];
        let w = CARD_W * scale;
        let h = CARD_H * scale;
        let rect = Rect::from_xywh(x - (w - CARD_W) / 2.0, y - (h - CARD_H) / 2.0, w, h);
        canvas.save();
        if let Some(elapsed) = f.move_elapsed {
            // Stable phases keep tiles from rocking in unison or jumping when reordered.
            let phase = f.catalog.apps[i].id.bytes().fold(0u32, |hash, byte| {
                hash.wrapping_mul(31).wrapping_add(byte as u32)
            });
            let offset = (phase & 255) as f32 / 256.0 * std::f32::consts::TAU;
            let frequency = 2.6 + ((phase >> 8) & 255) as f32 / 512.0;
            let angle = (elapsed * std::f32::consts::TAU * frequency + offset).sin()
                * 1.1
                * (elapsed / 0.12).min(1.0);
            canvas.rotate(angle, Some((rect.center_x(), rect.center_y()).into()));
        }
        if f.catalog.apps[i].input {
            let [r, g, b] = f.catalog.apps[i].color;
            let background = Color::from_rgb(r, g, b);
            let foreground = Color::from_rgb(224, 224, 224);
            canvas.save();
            canvas.translate((rect.left(), rect.top()));
            canvas.scale((scale, scale));
            canvas.draw_round_rect(
                Rect::from_wh(CARD_W, CARD_H),
                15.0,
                15.0,
                &paint(background),
            );
            let mut stroke = paint(foreground);
            stroke
                .set_style(skia_safe::paint::Style::Stroke)
                .set_stroke_width(3.0)
                .set_stroke_join(skia_safe::paint::Join::Round)
                .set_stroke_cap(skia_safe::paint::Cap::Round);
            if f.catalog.apps[i].id.to_ascii_lowercase().contains("hdmi") {
                // A solid connector silhouette with rounded shoulders and a recessed socket.
                let mut port = skia_safe::PathBuilder::new();
                port.move_to((92.0, 28.0))
                    .line_to((158.0, 28.0))
                    .quad_to((166.0, 28.0), (166.0, 36.0))
                    .line_to((166.0, 54.0))
                    .quad_to((166.0, 58.0), (163.0, 61.0))
                    .line_to((153.0, 73.0))
                    .quad_to((151.0, 76.0), (147.0, 76.0))
                    .line_to((103.0, 76.0))
                    .quad_to((99.0, 76.0), (97.0, 73.0))
                    .line_to((87.0, 61.0))
                    .quad_to((84.0, 58.0), (84.0, 54.0))
                    .line_to((84.0, 36.0))
                    .quad_to((84.0, 28.0), (92.0, 28.0))
                    .close();
                canvas.draw_path(&port.detach(), &paint(foreground));
                canvas.draw_round_rect(
                    Rect::from_xywh(96.0, 39.0, 58.0, 17.0),
                    4.0,
                    4.0,
                    &paint(background),
                );
                for n in 0..7 {
                    let x = 102.0 + n as f32 * 7.0;
                    canvas.draw_round_rect(
                        Rect::from_xywh(x, 39.0, 3.0, 7.0),
                        1.5,
                        1.5,
                        &paint(foreground),
                    );
                }
            } else {
                canvas.draw_round_rect(Rect::from_xywh(97.0, 28.0, 56.0, 38.0), 5.0, 5.0, &stroke);
                canvas.draw_line((114.0, 76.0), (136.0, 76.0), &stroke);
                canvas.draw_line((125.0, 66.0), (125.0, 76.0), &stroke);
            }
            let label_size = 19.2;
            let label = self.ellipsized(&f.catalog.apps[i].title, CARD_W - 32.0, label_size);
            self.text(
                canvas,
                &label,
                CARD_W / 2.0,
                117.0,
                label_size,
                foreground,
                true,
            );
            canvas.restore();
        } else if let Some(image) = &self.icons[i] {
            canvas.draw_image_rect_with_sampling_options(
                image,
                None,
                rect,
                skia_safe::FilterMode::Linear,
                &paint(Color::WHITE),
            );
        } else {
            let [r, g, b] = f.catalog.apps[i].color;
            canvas.draw_round_rect(
                rect,
                15.0,
                15.0,
                &paint(Color::from_rgb(r / 2, g / 2, b / 2)),
            );
            self.text(
                canvas,
                &f.catalog.apps[i].title,
                x + CARD_W / 2.0,
                y + 86.0,
                25.0,
                Color::WHITE,
                true,
            );
        }
        canvas.restore();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn jpeg_artwork_decodes_even_when_tv_names_it_png() {
        let path = std::env::temp_dir().join(format!(
            "tvhome-image-{}-{}.png",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let mut file = std::fs::File::create(&path).unwrap();
        image::RgbImage::new(2, 2)
            .write_to(&mut file, image::ImageFormat::Jpeg)
            .unwrap();
        drop(file);
        let app = App {
            id: "fixture".into(),
            title: "Fixture".into(),
            artwork: path.file_name().unwrap().to_str().unwrap().into(),
            contain: true,
            color: [0, 0, 0],
            background: None,
            input: false,
        };
        let (_, result) = decode_icons(path.parent().unwrap(), &[app]).recv().unwrap();
        std::fs::remove_file(path).unwrap();
        let image = result.unwrap();
        assert_eq!((image.width(), image.height()), (300, 180));
    }
}
