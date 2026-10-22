use super::{paint, Frame, Renderer, FOCUS_INK, FOCUS_SURFACE};
use crate::model::{weather_action_bounds, weather_bounds, weather_focus_bounds, LEFT};
use skia_safe::{Canvas, Color, Rect};

impl Renderer {
    fn weather_temperature(
        &self,
        canvas: &Canvas,
        text: &str,
        bounds: Rect,
        size: f32,
        color: Color,
    ) {
        let mut font = self.font.clone();
        font.set_size(size);
        let width = font.measure_str(text, None).0;
        let size = size * (bounds.width() / width.max(1.0)).min(1.0);
        self.text(
            canvas,
            text,
            bounds.left(),
            bounds.bottom(),
            size,
            color,
            false,
        );
    }
    pub(super) fn weather_summary(&self, canvas: &Canvas, f: &Frame<'_>) {
        let state = f.weather;
        let (x, y, _, h) = weather_bounds(f.catalog.apps.len());
        let compact = h < 200.0;
        let focused = f.weather_focused && !f.weather_details && f.city_picker.is_none();
        let foreground = if focused { FOCUS_INK } else { Color::WHITE };
        let muted = if focused {
            Color::from_rgb(76, 78, 87)
        } else {
            Color::from_rgb(166, 166, 175)
        };
        canvas.save();
        if focused {
            let (left, top, width, height) = weather_focus_bounds(f.catalog.apps.len());
            let rect = Rect::from_xywh(left, top, width, height);
            self.shadow(canvas, rect, 0.65, 8.0);
            canvas.draw_round_rect(rect, 26.0, 26.0, &paint(FOCUS_SURFACE));
        }
        let city = self.ellipsized(
            state.city.as_deref().unwrap_or(if state.refreshing {
                "Finding your city…"
            } else {
                "Local weather"
            }),
            if compact { 230.0 } else { 400.0 },
            if compact { 22.0 } else { 29.0 },
        );
        self.text(
            canvas,
            &city,
            x,
            y + 46.0,
            if compact { 22.0 } else { 29.0 },
            if focused { foreground } else { muted },
            false,
        );
        let baseline = y + if compact { 130.0 } else { 172.0 };
        let temp = state
            .conditions
            .as_ref()
            .map(|c| format!("{:.0}°", c.temperature))
            .unwrap_or_else(|| "—°".into());
        self.weather_temperature(
            canvas,
            &temp,
            Rect::from_xywh(
                x,
                baseline - 150.0,
                if compact { 130.0 } else { 195.0 },
                150.0,
            ),
            if compact { 64.0 } else { 125.0 },
            foreground,
        );
        let icon_size = if compact { 55.0 } else { 104.0 };
        if let Some(c) = &state.conditions {
            self.symbols.draw(
                canvas,
                c.icon(),
                x + if compact { 150.0 } else { 215.0 },
                baseline - icon_size,
                icon_size,
                if focused {
                    foreground
                } else {
                    Color::from_rgb(225, 225, 231)
                },
            );
            if !compact {
                let mut description = c.description().to_string();
                if let (Some(high), Some(low)) = (c.high, c.low) {
                    description.push_str(&format!(" · H:{high:.0}° L:{low:.0}°"));
                }
                self.text(
                    canvas,
                    &self.ellipsized(&description, 365.0, 25.0),
                    x,
                    y + 228.0,
                    25.0,
                    if focused {
                        muted
                    } else {
                        Color::from_rgb(199, 199, 207)
                    },
                    false,
                );
            }
        } else if !compact {
            self.text(
                canvas,
                if state.refreshing {
                    "Updating…"
                } else {
                    "Weather unavailable"
                },
                x,
                y + 228.0,
                25.0,
                muted,
                false,
            );
        }
        let ax = x + if compact { 325.0 } else { 445.0 };
        canvas.draw_rect(
            Rect::from_xywh(
                ax - 64.0,
                y + 20.0,
                1.0,
                if compact { 124.0 } else { 208.0 },
            ),
            &paint(if focused {
                Color::from_rgb(193, 195, 204)
            } else {
                Color::from_rgb(55, 55, 61)
            }),
        );
        self.text(
            canvas,
            "AQI",
            ax,
            y + 46.0,
            if compact { 20.0 } else { 23.0 },
            muted,
            false,
        );
        let number = state
            .air
            .as_ref()
            .map(|a| a.number())
            .unwrap_or_else(|| "—".into());
        self.text(
            canvas,
            &number,
            ax,
            y + if compact { 98.0 } else { 170.0 },
            if compact { 38.0 } else { 67.0 },
            foreground,
            false,
        );
        let (label, color) = state
            .air
            .as_ref()
            .map(|a| {
                let [r, g, b] = a.color();
                (
                    a.label(),
                    if focused {
                        Color::from_rgb(r / 2, g / 2, b / 2)
                    } else {
                        Color::from_rgb(r, g, b)
                    },
                )
            })
            .unwrap_or(("Unavailable", muted));
        self.text(
            canvas,
            label,
            ax,
            y + if compact { 133.0 } else { 218.0 },
            if compact { 20.0 } else { 23.0 },
            color,
            false,
        );
        if !compact && (state.problem.is_some() || state.sample) {
            self.text(canvas, &state.age_label(), x, y + 258.0, 18.0, muted, false);
        }
        canvas.restore();
    }

    pub(super) fn weather_panel(&self, canvas: &Canvas, f: &Frame<'_>) {
        let state = f.weather;
        let x = LEFT + 40.0;
        let muted = Color::from_rgb(166, 166, 177);
        canvas.draw_rect(
            Rect::from_wh(1920.0, 1080.0),
            &paint(Color::from_argb(160, 0, 0, 0)),
        );
        canvas.draw_round_rect(
            Rect::from_xywh(LEFT, 164.0, 848.0, 760.0),
            30.0,
            30.0,
            &paint(Color::from_rgb(36, 37, 44)),
        );
        let city = self.ellipsized(
            state.city.as_deref().unwrap_or("Local weather"),
            740.0,
            32.0,
        );
        self.text(canvas, &city, x, 222.0, 32.0, Color::WHITE, false);
        let location_label = if let Some(place) = f.weather_location {
            format!("Selected city · {}", place.detail())
        } else if state.sample {
            "Sample location".into()
        } else {
            "Approximate location from your public IP".into()
        };
        self.text(
            canvas,
            &self.ellipsized(&location_label, 740.0, 21.0),
            x,
            260.0,
            21.0,
            muted,
            false,
        );
        let temp = state
            .conditions
            .as_ref()
            .map(|c| format!("{:.0}°", c.temperature))
            .unwrap_or_else(|| "—°".into());
        self.weather_temperature(
            canvas,
            &temp,
            Rect::from_xywh(x, 280.0, 144.0, 80.0),
            78.0,
            Color::WHITE,
        );
        if let Some(c) = &state.conditions {
            self.symbols
                .draw(canvas, c.icon(), x + 160.0, 293.0, 72.0, Color::WHITE);
            self.text(canvas, c.description(), x, 410.0, 26.0, Color::WHITE, false);
            if let Some(feels) = c.feels_like {
                self.text(
                    canvas,
                    &format!("Feels like {feels:.0}°"),
                    x,
                    449.0,
                    23.0,
                    muted,
                    false,
                );
            }
            if let (Some(high), Some(low)) = (c.high, c.low) {
                self.text(
                    canvas,
                    &format!("H:{high:.0}° · L:{low:.0}°"),
                    x,
                    483.0,
                    23.0,
                    muted,
                    false,
                );
            }
        } else {
            self.text(canvas, "Weather unavailable", x, 410.0, 24.0, muted, false);
        }
        let ax = x + 414.0;
        canvas.draw_rect(
            Rect::from_xywh(ax - 36.0, 294.0, 1.0, 193.0),
            &paint(Color::from_rgb(65, 65, 75)),
        );
        self.text(
            canvas,
            "AQI · model estimate",
            ax,
            312.0,
            22.0,
            muted,
            false,
        );
        self.text(
            canvas,
            &state
                .air
                .as_ref()
                .map(|a| a.number())
                .unwrap_or_else(|| "—".into()),
            ax,
            390.0,
            72.0,
            Color::WHITE,
            false,
        );
        if let Some(a) = &state.air {
            let [r, g, b] = a.color();
            self.text(
                canvas,
                a.label(),
                ax,
                438.0,
                26.0,
                Color::from_rgb(r, g, b),
                false,
            );
            self.text(
                canvas,
                &format!("Main pollutant: {}", a.pollutant),
                ax,
                478.0,
                21.0,
                muted,
                false,
            );
        } else {
            self.text(canvas, "Unavailable", ax, 438.0, 24.0, muted, false);
        }
        canvas.draw_rect(
            Rect::from_xywh(x, 515.0, 768.0, 1.0),
            &paint(Color::from_rgb(65, 65, 75)),
        );
        self.text(canvas, &state.age_label(), x, 550.0, 22.0, muted, false);
        self.text(
            canvas,
            state
                .problem
                .unwrap_or("Weather: Open-Meteo · Air quality: CAMS via Open-Meteo"),
            x,
            584.0,
            20.0,
            muted,
            false,
        );
        if state.problem.is_some() {
            self.text(
                canvas,
                if f.weather_location.is_some() {
                    "Open-Meteo / CAMS · City search: GeoNames"
                } else {
                    "Open-Meteo / CAMS · Location: IPWhois"
                },
                x,
                615.0,
                18.0,
                muted,
                false,
            );
        }
        for (i, label) in [
            "Change city",
            if state.refreshing {
                "Updating…"
            } else {
                "Refresh"
            },
            "Close",
        ]
        .iter()
        .enumerate()
        {
            self.panel_button(
                canvas,
                label,
                weather_action_bounds(i),
                f.weather_selection == i,
            );
        }
    }

    pub(super) fn city_panel(
        &self,
        canvas: &Canvas,
        f: &Frame<'_>,
        picker: &crate::city::Picker,
        selected: usize,
    ) {
        use crate::city;
        let muted = Color::from_rgb(166, 166, 177);
        canvas.draw_rect(
            Rect::from_wh(1920.0, 1080.0),
            &paint(Color::from_argb(190, 0, 0, 0)),
        );
        canvas.draw_round_rect(
            Rect::from_xywh(100.0, 164.0, 1720.0, 728.0),
            30.0,
            30.0,
            &paint(Color::from_rgb(36, 37, 44)),
        );
        self.text(
            canvas,
            "Choose city",
            140.0,
            222.0,
            32.0,
            Color::WHITE,
            false,
        );
        let current = f
            .weather_location
            .map(|p| format!("{} · {}", p.city, p.detail()))
            .unwrap_or_else(|| {
                format!(
                    "Automatic location · {}",
                    f.weather.city.as_deref().unwrap_or("Detecting…")
                )
            });
        self.text(
            canvas,
            &self.ellipsized(&current, 1320.0, 21.0),
            140.0,
            256.0,
            21.0,
            muted,
            false,
        );
        canvas.draw_round_rect(
            Rect::from_xywh(140.0, 276.0, 712.0, 68.0),
            14.0,
            14.0,
            &paint(Color::from_rgb(16, 17, 22)),
        );
        self.text(
            canvas,
            &self.ellipsized(
                if picker.query.is_empty() {
                    "Enter city name"
                } else {
                    &picker.query
                },
                664.0,
                28.0,
            ),
            164.0,
            320.0,
            28.0,
            if picker.query.is_empty() {
                muted
            } else {
                Color::WHITE
            },
            false,
        );
        self.text(canvas, "Results", 936.0, 316.0, 26.0, Color::WHITE, false);
        for index in 0..city::RESULTS + picker.results.len() {
            let (x, y, w, h) = city::bounds(index);
            let focused = index == selected;
            canvas.draw_round_rect(
                Rect::from_xywh(x, y, w, h),
                12.0,
                12.0,
                &paint(if focused {
                    Color::from_rgb(223, 224, 230)
                } else {
                    Color::from_rgb(49, 50, 60)
                }),
            );
            let color = if focused { Color::BLACK } else { Color::WHITE };
            if index >= city::RESULTS {
                let place = &picker.results[index - city::RESULTS];
                self.text(
                    canvas,
                    &self.ellipsized(&place.city, w - 32.0, 26.0),
                    x + 16.0,
                    y + 33.0,
                    26.0,
                    color,
                    false,
                );
                self.text(
                    canvas,
                    &self.ellipsized(&place.detail(), w - 32.0, 20.0),
                    x + 16.0,
                    y + 65.0,
                    20.0,
                    if focused {
                        Color::from_rgb(60, 61, 70)
                    } else {
                        muted
                    },
                    false,
                );
            } else {
                let label = match index {
                    0..=29 => (city::KEYS.as_bytes()[index] as char).to_string(),
                    city::SPACE => "Space".into(),
                    city::DELETE => "Delete".into(),
                    city::CLEAR => "Clear".into(),
                    city::SEARCH => "Search".into(),
                    city::AUTO => "Use automatic location".into(),
                    _ => "Back".into(),
                };
                self.text(
                    canvas,
                    &label,
                    x + w / 2.0,
                    y + h / 2.0 + 9.0,
                    if index < city::SPACE { 28.0 } else { 23.0 },
                    color,
                    true,
                );
            }
        }
        let message = if picker.saving {
            "Saving city…"
        } else if picker.searching {
            "Searching…"
        } else {
            picker
                .message
                .unwrap_or("Search, then choose a city from the results.")
        };
        if picker.results.is_empty() {
            self.wrapped_text(
                canvas,
                message,
                Rect::from_xywh(936.0, 360.0, 760.0, 160.0),
                muted,
            );
        } else if picker.saving {
            self.text(canvas, message, 936.0, 858.0, 22.0, muted, false);
        }
        self.text(
            canvas,
            "Use the remote or type with a keyboard.",
            140.0,
            708.0,
            21.0,
            muted,
            false,
        );
        self.text(
            canvas,
            "City search: Open-Meteo / GeoNames",
            140.0,
            858.0,
            19.0,
            muted,
            false,
        );
    }
}
