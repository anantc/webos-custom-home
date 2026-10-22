mod audio;
mod catalog;
mod city;
mod gl;
mod model;
mod platform;
mod preferences;
mod render;
mod speedtest;
mod symbols;
mod weather;

use anyhow::{Context, Result};
use model::*;
use platform::{cursor, wifi};
use sdl3::event::{Event, WindowEvent};
use sdl3::keyboard::Keycode;
use std::path::PathBuf;
use std::time::{Duration, Instant};

#[derive(Clone, Copy, PartialEq, Debug)]
enum Panel {
    Controls,
    Inputs,
    Wifi,
    SpeedTest,
    Weather,
    City,
}
#[derive(Clone, Copy, PartialEq, Debug)]
enum FocusTarget {
    Header(Header),
    App(usize),
    Menu(Panel, usize),
    Pin(usize),
}
#[derive(Clone, Copy, PartialEq, Debug)]
enum Header {
    Wifi,
    Controls,
    Weather,
}
#[derive(Debug, PartialEq)]
enum Action {
    Launch(String, String),
    TogglePin(String),
    SaveOrder(Vec<String>),
    SpeedTest,
    RefreshWeather,
    SearchCity(u64, String),
    ChooseCity(Option<weather::Location>),
}
const MOVE_HOLD: Duration = Duration::from_millis(700);
#[derive(Clone, Copy, PartialEq)]
enum ConfirmButton {
    Key,
    Pointer,
}
struct ConfirmPress {
    button: ConfirmButton,
    target: Option<FocusTarget>,
    app_id: Option<String>,
    at: Instant,
    consumed: bool,
}
struct TileMove {
    started: Instant,
    id: String,
    order: Vec<String>,
    saving: bool,
}
struct Ui {
    selected: usize,
    header: Option<Header>,
    panel: Option<Panel>,
    panel_index: usize,
    pin_focused: bool,
    scroll: Tween,
    scales: Vec<Tween>,
    positions: Vec<TileMotion>,
    toast: Option<(String, Instant)>,
    metrics: bool,
    quit: bool,
    confirm: Option<ConfirmPress>,
    moving: Option<TileMove>,
    order_changed: bool,
    city: city::Picker,
}
impl Ui {
    fn new(count: usize) -> Self {
        let mut scales = vec![Tween::new(1.0); count];
        scales[0] = Tween::new(1.14);
        Self {
            selected: 0,
            header: None,
            panel: None,
            panel_index: 0,
            pin_focused: false,
            scroll: Tween::new(0.0),
            scales,
            positions: (0..count)
                .map(|i| TileMotion::new(position(i, count)))
                .collect(),
            toast: None,
            metrics: false,
            quit: false,
            confirm: None,
            moving: None,
            order_changed: false,
            city: city::Picker::default(),
        }
    }
    fn reconcile(&mut self, previous: &Catalog, updated: &Catalog) {
        if self
            .moving
            .as_ref()
            .is_some_and(|m| !m.saving && !updated.apps.iter().any(|a| a.id == m.id))
        {
            self.cancel_move();
            self.toast = Some(("That tile is no longer available.".into(), Instant::now()));
        }
        let id = previous.apps.get(self.selected).map(|app| app.id.as_str());
        self.selected = updated
            .apps
            .iter()
            .position(|app| Some(app.id.as_str()) == id)
            .unwrap_or_else(|| self.selected.min(updated.apps.len() - 1));
        let now = Instant::now();
        let previous_indices: std::collections::HashMap<_, _> = previous
            .apps
            .iter()
            .enumerate()
            .map(|(i, app)| (app.id.as_str(), i))
            .collect();
        self.positions = updated
            .apps
            .iter()
            .enumerate()
            .map(|(i, app)| {
                let destination = position(i, updated.apps.len());
                let mut motion = previous_indices
                    .get(app.id.as_str())
                    .map(|old| self.positions[*old])
                    .unwrap_or_else(|| TileMotion::new(destination));
                motion.retarget(destination, now);
                motion
            })
            .collect();
        self.scales = updated
            .apps
            .iter()
            .enumerate()
            .map(|(i, app)| {
                let target = if i == self.selected && self.header.is_none() && self.panel.is_none()
                {
                    1.14
                } else {
                    1.0
                };
                let mut scale = previous_indices
                    .get(app.id.as_str())
                    .map(|old| self.scales[*old])
                    .unwrap_or_else(|| Tween::new(target));
                scale.set(target);
                scale
            })
            .collect();
        self.scroll.set(scroll_for(
            self.selected,
            updated.apps.len(),
            self.scroll.to,
        ));
        self.panel_index = self
            .panel_index
            .min(self.items(updated).len().saturating_sub(1));
    }
    fn items(&self, catalog: &Catalog) -> Vec<String> {
        if self.panel == Some(Panel::City) {
            return vec![String::new(); city::RESULTS + self.city.results.len()];
        }
        if self.panel == Some(Panel::Weather) {
            return vec!["Change city".into(), "Refresh".into(), "Close".into()];
        }
        if self.panel == Some(Panel::Wifi) {
            return vec!["Speed test".into(), "Close".into()];
        }
        if self.panel == Some(Panel::SpeedTest) {
            return vec!["Run speed test".into(), "Back".into()];
        }
        if self.panel == Some(Panel::Inputs) {
            catalog.input_items().into_iter().map(|i| i.label).collect()
        } else {
            vec!["Inputs".into(), "TV settings".into(), "Exit".into()]
        }
    }
    fn focus_target(&self) -> Option<FocusTarget> {
        if self.panel == Some(Panel::Inputs) && self.pin_focused {
            Some(FocusTarget::Pin(self.panel_index))
        } else if let Some(panel) = self.panel {
            Some(FocusTarget::Menu(panel, self.panel_index))
        } else if let Some(header) = self.header {
            Some(FocusTarget::Header(header))
        } else {
            Some(FocusTarget::App(self.selected))
        }
    }
    fn select(&mut self, index: usize, count: usize) {
        self.selected = index;
        self.header = None;
        self.scroll.set(scroll_for(index, count, self.scroll.to));
    }
    fn direction(&mut self, direction: Direction, catalog: &Catalog) {
        if self.panel == Some(Panel::City) {
            self.confirm = None;
            self.panel_index = city::navigate(self.panel_index, direction, self.city.results.len());
            return;
        }
        if let Some(moving) = &mut self.moving {
            if !moving.saving {
                let destination =
                    next(self.selected, direction, catalog.apps.len()).unwrap_or(self.selected);
                if destination != self.selected {
                    let mut order = preferences::tile_ids(catalog);
                    let id = order.remove(self.selected);
                    order.insert(destination, id);
                    moving.order = order;
                    self.order_changed = true;
                }
            }
            return;
        }
        self.confirm = None;
        if self.panel.is_some() {
            let count = self.items(catalog).len();
            match direction {
                Direction::Up => self.panel_index = self.panel_index.saturating_sub(1),
                Direction::Down => self.panel_index = (self.panel_index + 1).min(count - 1),
                Direction::Left if self.panel == Some(Panel::Inputs) && self.pin_focused => {
                    self.pin_focused = false
                }
                Direction::Right if self.panel == Some(Panel::Inputs) => self.pin_focused = true,
                Direction::Left => self.back(),
                _ => (),
            }
        } else if let Some(header) = self.header {
            match direction {
                Direction::Down => self.header = None,
                Direction::Left if header == Header::Wifi => self.header = Some(Header::Controls),
                Direction::Left if header == Header::Controls => {
                    self.header = Some(Header::Weather)
                }
                Direction::Right if header == Header::Weather => {
                    self.header = Some(Header::Controls)
                }
                Direction::Right if header == Header::Controls => self.header = Some(Header::Wifi),
                _ => (),
            }
        } else if let Some(next) = next(self.selected, direction, catalog.apps.len()) {
            self.select(next, catalog.apps.len());
        } else {
            self.header = Some(match self.selected % COLS {
                0..=2 => Header::Weather,
                3..=4 => Header::Controls,
                _ => Header::Wifi,
            });
        }
    }
    fn back(&mut self) {
        self.confirm = None;
        if self.moving.is_some() {
            self.cancel_move();
            return;
        }
        self.pin_focused = false;
        if self.panel == Some(Panel::City) {
            self.city.cancel();
            self.panel = Some(Panel::Weather);
            self.panel_index = 0;
        } else if self.panel == Some(Panel::Weather) {
            self.panel = None;
            self.header = Some(Header::Weather);
        } else if self.panel == Some(Panel::SpeedTest) {
            self.panel = Some(Panel::Wifi);
            self.panel_index = 0;
        } else if self.panel == Some(Panel::Wifi) {
            self.panel = None;
            self.header = Some(Header::Wifi);
        } else if self.panel == Some(Panel::Inputs) {
            self.panel = Some(Panel::Controls);
            self.panel_index = 0;
        } else if self.panel.is_some() {
            self.panel = None;
        } else if self.header.is_some() {
            self.header = None;
        } else {
            self.quit = true;
        }
    }
    fn activate(&mut self, catalog: &Catalog) -> Option<Action> {
        if self.panel == Some(Panel::City) {
            if self.city.saving {
                return None;
            }
            match self.panel_index {
                0..=29 => self
                    .city
                    .type_text(&(city::KEYS.as_bytes()[self.panel_index] as char).to_string()),
                city::SPACE => self.city.type_text(" "),
                city::DELETE => self.city.delete(),
                city::CLEAR => self.city.clear(),
                city::SEARCH => {
                    return self
                        .city
                        .begin()
                        .map(|(id, text)| Action::SearchCity(id, text))
                }
                city::AUTO => return Some(Action::ChooseCity(None)),
                city::BACK => self.back(),
                index => {
                    return self
                        .city
                        .results
                        .get(index - city::RESULTS)
                        .cloned()
                        .map(|p| Action::ChooseCity(Some(p)))
                }
            }
            return None;
        }
        if let Some(moving) = &self.moving {
            return (!moving.saving).then(|| Action::SaveOrder(preferences::tile_ids(catalog)));
        }
        if self.panel == Some(Panel::Weather) {
            if self.panel_index == 0 {
                self.city.clear();
                self.panel = Some(Panel::City);
                self.panel_index = 0;
                return None;
            }
            if self.panel_index == 1 {
                return Some(Action::RefreshWeather);
            }
            self.back();
            return None;
        }
        if self.panel == Some(Panel::SpeedTest) {
            if self.panel_index == 0 {
                return Some(Action::SpeedTest);
            }
            self.back();
            return None;
        }
        if self.panel == Some(Panel::Wifi) {
            if self.panel_index == 0 {
                self.panel = Some(Panel::SpeedTest);
                self.panel_index = 0;
            } else {
                self.back();
            }
            return None;
        }
        if self.panel == Some(Panel::Inputs) {
            let input = catalog.input_items().get(self.panel_index)?.clone();
            if self.pin_focused {
                return Some(Action::TogglePin(input.app_id));
            }
            self.panel = None;
            return Some(Action::Launch(input.app_id, input.label));
        }
        if self.panel == Some(Panel::Controls) {
            match self.panel_index {
                0 => {
                    self.panel = Some(Panel::Inputs);
                    self.panel_index = 0;
                    self.pin_focused = false;
                }
                1 => {
                    self.panel = None;
                    return Some(Action::Launch(
                        "com.palm.app.settings".into(),
                        "TV settings".into(),
                    ));
                }
                _ => self.quit = true,
            }
        } else if let Some(header) = self.header {
            self.panel = Some(match header {
                Header::Wifi => Panel::Wifi,
                Header::Controls => Panel::Controls,
                Header::Weather => Panel::Weather,
            });
            self.panel_index = 0;
        } else {
            let app = &catalog.apps[self.selected];
            return Some(Action::Launch(app.id.clone(), app.title.clone()));
        }
        None
    }
    fn pointer(&mut self, x: f32, y: f32, catalog: &Catalog) -> bool {
        if self.panel == Some(Panel::City) {
            for index in 0..city::RESULTS + self.city.results.len() {
                let (left, top, w, h) = city::bounds(index);
                if (left..left + w).contains(&x) && (top..top + h).contains(&y) {
                    self.panel_index = index;
                    return true;
                }
            }
            return false;
        }
        if self.moving.is_some() {
            // Pointer movement must not drop or change the tile being moved with the arrows.
            let (left, top) = self.positions[self.selected].value_at(Instant::now());
            return (left..left + CARD_W).contains(&x)
                && (top - self.scroll.value()..top - self.scroll.value() + CARD_H).contains(&y);
        }
        if matches!(
            self.panel,
            Some(Panel::Wifi | Panel::SpeedTest | Panel::Weather)
        ) {
            for i in 0..if self.panel == Some(Panel::Weather) {
                3
            } else {
                2
            } {
                let (left, top, width, height) = if self.panel == Some(Panel::Weather) {
                    weather_action_bounds(i)
                } else if self.panel == Some(Panel::Wifi) {
                    wifi_action_bounds(i)
                } else {
                    speed_action_bounds(i)
                };
                if (left..left + width).contains(&x) && (top..top + height).contains(&y) {
                    self.panel_index = i;
                    return true;
                }
            }
            return false;
        }
        if self.panel.is_some() {
            if (SETTINGS_PANEL_LEFT + 24.0..SETTINGS_PANEL_LEFT + 576.0).contains(&x)
                && y >= PANEL_ROW_TOP
            {
                let i = ((y - PANEL_ROW_TOP) / PANEL_ROW_STEP) as usize;
                if i < self.items(catalog).len() && (y - PANEL_ROW_TOP) % PANEL_ROW_STEP < 76.0 {
                    if self.panel == Some(Panel::Inputs) && (PIN_LEFT - 12.0..PIN_LEFT).contains(&x)
                    {
                        return false;
                    }
                    self.pin_focused = self.panel == Some(Panel::Inputs) && x >= PIN_LEFT;
                    self.panel_index = i;
                    return true;
                }
            }
            return false;
        }
        for (header, (left, top, width, height)) in [
            (Header::Wifi, wifi_bounds()),
            (Header::Controls, status_bounds()),
            (Header::Weather, weather_bounds(catalog.apps.len())),
        ] {
            if header == Header::Weather && self.scroll.value() > 1.0 {
                continue;
            }
            if (left..left + width).contains(&x) && (top..top + height).contains(&y) {
                self.header = Some(header);
                return true;
            }
        }
        for i in 0..catalog.apps.len() {
            let (cx, cy) = self.positions[i].value_at(Instant::now());
            let cy = cy - self.scroll.value();
            if (cx..cx + CARD_W).contains(&x) && (cy..cy + CARD_H).contains(&y) {
                self.select(i, catalog.apps.len());
                return true;
            }
        }
        false
    }

    fn confirm_down(&mut self, button: ConfirmButton, now: Instant, catalog: &Catalog) {
        if self.confirm.is_none() {
            self.confirm = Some(ConfirmPress {
                button,
                target: self.focus_target(),
                app_id: (self.panel.is_none() && self.header.is_none())
                    .then(|| catalog.apps[self.selected].id.clone()),
                at: now,
                consumed: false,
            });
        }
    }
    fn same_press_target(&self, press: &ConfirmPress, catalog: &Catalog) -> bool {
        if let Some(id) = &press.app_id {
            self.panel.is_none() && self.header.is_none() && catalog.apps[self.selected].id == *id
        } else {
            self.focus_target() == press.target
        }
    }
    fn hold_tick(&mut self, now: Instant, catalog: &Catalog, writable: bool) -> bool {
        let Some(press) = &self.confirm else {
            return false;
        };
        if !writable
            || self.moving.is_some()
            || press.consumed
            || press.app_id.is_none()
            || now.saturating_duration_since(press.at) < MOVE_HOLD
            || !self.same_press_target(press, catalog)
        {
            return false;
        }
        self.moving = Some(TileMove {
            started: now,
            id: catalog.apps[self.selected].id.clone(),
            order: preferences::tile_ids(catalog),
            saving: false,
        });
        self.confirm.as_mut().unwrap().consumed = true;
        self.toast = None;
        true
    }
    fn confirm_up(
        &mut self,
        button: ConfirmButton,
        now: Instant,
        catalog: &Catalog,
        writable: bool,
    ) -> Option<Action> {
        if self
            .confirm
            .as_ref()
            .is_none_or(|press| press.button != button)
        {
            return None;
        }
        self.hold_tick(now, catalog, writable);
        let press = self.confirm.take()?;
        if press.consumed
            || !self.same_press_target(&press, catalog)
            || (!writable
                && press.app_id.is_some()
                && now.saturating_duration_since(press.at) >= MOVE_HOLD)
        {
            return None;
        }
        self.activate(catalog)
    }
    fn cancel_move(&mut self) {
        self.confirm = None;
        if self.moving.as_ref().is_some_and(|m| !m.saving) {
            self.moving = None;
            self.order_changed = true;
        }
    }
    fn save_finished(&mut self, success: bool) {
        if let Some(moving) = &mut self.moving {
            if success {
                self.moving = None;
                self.order_changed = true;
                self.confirm = None;
            } else {
                moving.saving = false;
            }
        }
    }
    fn displayed_catalog(&self, prefs: &preferences::Preferences, source: &Catalog) -> Catalog {
        let mut catalog = prefs.catalog(source);
        if let Some(moving) = &self.moving {
            preferences::apply_order(&mut catalog, &moving.order);
        }
        catalog
    }
    fn refresh_catalog(
        &mut self,
        catalog: &mut Catalog,
        prefs: &preferences::Preferences,
        source: &Catalog,
    ) {
        let mut updated = self.displayed_catalog(prefs, source);
        let was_moving = self.moving.is_some();
        self.reconcile(catalog, &updated);
        if was_moving && self.moving.is_none() {
            // If the picked tile disappeared, discard its draft order as well.
            let restored = prefs.catalog(source);
            self.reconcile(&updated, &restored);
            updated = restored;
        }
        *catalog = updated;
        self.order_changed = false;
    }
}

fn refresh_grid(
    ui: &mut Ui,
    catalog: &mut Catalog,
    prefs: &preferences::Preferences,
    source: &Catalog,
    renderer: &mut render::Renderer,
    icons: &[Option<skia_safe::Image>],
) {
    ui.refresh_catalog(catalog, prefs, source);
    renderer.icons = catalog::icons_for(source, icons, catalog);
}

fn main() -> Result<()> {
    tracing_subscriber::fmt().with_target(false).init();
    let args: Vec<String> = std::env::args().collect();
    let preview = args.iter().any(|s| s == "--preview");
    let assets = if let Some(i) = args.iter().position(|s| s == "--assets") {
        PathBuf::from(args.get(i + 1).context("--assets requires a directory")?)
    } else {
        std::env::current_exe()?
            .parent()
            .context("binary directory")?
            .parent()
            .context("package directory")?
            .join("assets")
    };
    let initial_catalog = std::fs::read(assets.join("catalog.json"))?;
    let mut source_catalog: Catalog = serde_json::from_slice(&initial_catalog)?;
    source_catalog.validate(preview)?;
    let preferences_path = if preview {
        assets
            .parent()
            .context("assets directory")?
            .join("state/preview-preferences.json")
    } else {
        assets
            .parent()
            .context("app directory")?
            .join("state/preferences.json")
    };
    let mut prefs = preferences::Preferences::load(&preferences_path).unwrap_or_else(|error| {
        tracing::warn!(%error, "Could not read Home preferences");
        preferences::Preferences::default()
    });
    let result_path = preferences_path.with_file_name(if preview {
        "preview-speedtest-result.json"
    } else {
        "speedtest-result.json"
    });
    let mut speed_test = speedtest::Monitor::new(result_path, preview);
    let preference_writer = preferences::writer(preferences_path);
    let mut preference_pending = false;
    let mut catalog = prefs.catalog(&source_catalog);
    let catalog_worker = catalog::worker(assets.clone(), preview, initial_catalog);
    let mut catalog_check = Instant::now();
    let mut renderer = render::Renderer::new(&assets, catalog.apps.len())?;
    let decoded = render::decode_icons(&assets, &source_catalog.apps);
    let launcher = platform::luna::worker(preview);
    let wifi_worker = wifi::worker(preview);
    let mut wifi_monitor = wifi::Monitor::new(Instant::now());
    let mut weather_monitor = weather::Monitor::new(preview, prefs.weather_location.clone());
    let city_worker = city::worker();
    // Set before window creation; leave Home owned by webOS so it remains an escape route.
    sdl3::hint::set("SDL_WEBOS_ACCESS_POLICY_KEYS_BACK", "true");
    sdl3::hint::set("SDL_WEBOS_ACCESS_POLICY_KEYS_EXIT", "true");
    cursor::configure();
    let sdl = sdl3::init().map_err(anyhow::Error::msg)?;
    let video = sdl.video().map_err(anyhow::Error::msg)?;
    video.gl_attr().set_alpha_size(8);
    video.gl_attr().set_stencil_size(8);
    let window = video
        .window("Home", 1920, 1080)
        .opengl()
        .fullscreen()
        .build()?;
    // Release SDL's default inhibition so LG can manage its screensaver.
    match video.enable_screen_saver() {
        Ok(()) => tracing::info!(
            allowed = video.is_screen_saver_enabled(),
            "SDL screensaver inhibition released"
        ),
        Err(error) => tracing::warn!(%error, "Could not release SDL screensaver inhibition"),
    }
    let mut graphics = gl::ConsoleGl::new(&window, &video)?;
    graphics.set_swap_interval(&video, true);
    let mut sounds = audio::NavigationSound::new(&sdl);
    let mut events = sdl.event_pump().map_err(anyhow::Error::msg)?;
    let mut ui = Ui::new(catalog.apps.len());
    let mut dirty = true;
    let mut focused = true;
    let mut input_at: Option<Instant> = None;
    let mut clock = platform::time_label();
    let mut date = platform::clock(c"%A, %e %B");
    let mut clock_check = Instant::now();
    let mut wheel_at = Instant::now() - Duration::from_secs(1);
    let mut metric_text = String::new();
    let mut frame_count = 0u64;
    let mut draw_samples = Vec::<f64>::new();
    let mut last_metrics = Instant::now();

    // Decode before the first presentation and exercise the actual GPU drawing paths.
    // This warms image sampling, text and panel primitives without visible flashing.
    for (i, image) in decoded {
        match image {
            Ok(image) => renderer.icons[i] = Some(image),
            Err(e) => tracing::warn!(index = i, error = %e, "icon unavailable; using fallback"),
        }
    }
    let mut source_icons = renderer.icons[..source_catalog.apps.len()].to_vec();
    renderer.icons = catalog::icons_for(&source_catalog, &source_icons, &catalog);
    let warm_items = ui.items(&catalog);
    let scales: Vec<f32> = ui.scales.iter().map(Tween::value).collect();
    let positions: Vec<_> = ui
        .positions
        .iter()
        .map(|p| p.value_at(Instant::now()))
        .collect();
    {
        let size = window.size();
        let canvas = graphics.surface(size.0, size.1)?.canvas();
        renderer.draw(
            canvas,
            &render::Frame {
                catalog: &catalog,
                selected: 0,
                status: false,
                scroll: 0.0,
                scales: &scales,
                positions: &positions,
                clock: &clock,
                date: &date,
                panel: Some((&warm_items, 0)),
                input_pins: None,
                pin_focused: false,
                wifi: &wifi_monitor.status,
                wifi_focused: false,
                wifi_details: true,
                wifi_selection: 0,
                speed_test: None,
                weather: &weather_monitor.state,
                weather_focused: false,
                weather_details: false,
                weather_selection: 0,
                weather_location: prefs.weather_location.as_ref(),
                city_picker: None,
                move_elapsed: Some(0.08),
                move_hint: None,
                toast: Some("Home"),
                metrics: None,
            },
            size,
        );
    }
    graphics.flush();
    tracing::info!(
        preview,
        apps = catalog.apps.len(),
        "ready; timings measure SDL receipt to submission, not physical key-to-pixel latency"
    );

    while !ui.quit {
        let frame_start = Instant::now();
        for event in events.poll_iter() {
            if cursor::is_notification(&event) {
                continue;
            }
            let previous_target = ui.focus_target();
            let previous_panel = ui.panel;
            let navigation = matches!(
                &event,
                Event::MouseMotion { .. }
                    | Event::MouseWheel { .. }
                    | Event::KeyDown {
                        keycode: Some(Keycode::Left | Keycode::Right | Keycode::Up | Keycode::Down),
                        ..
                    }
            );
            let mut launch = None;
            let mut interacted = false;
            match event {
                Event::Quit { .. } => ui.quit = true,
                Event::Window { win_event, .. } => match win_event {
                    WindowEvent::FocusGained | WindowEvent::Shown | WindowEvent::Exposed => {
                        if !focused {
                            wifi_monitor.refresh(Instant::now());
                            catalog_check = Instant::now() - Duration::from_secs(6);
                        }
                        focused = true;
                        dirty = true;
                    }
                    WindowEvent::FocusLost | WindowEvent::Hidden | WindowEvent::Minimized => {
                        focused = false;
                        ui.cancel_move();
                        speed_test.cancel();
                        sounds.suspend();
                    }
                    _ => (),
                },
                Event::KeyDown {
                    keycode,
                    raw,
                    repeat,
                    ..
                } => {
                    interacted = true;
                    if raw == 0x19c && !repeat {
                        ui.back();
                    } else if (raw == 0x191 || keycode == Some(Keycode::F8)) && !repeat {
                        ui.metrics = !ui.metrics;
                    } else {
                        match keycode {
                            Some(Keycode::Left) => ui.direction(Direction::Left, &catalog),
                            Some(Keycode::Right) => ui.direction(Direction::Right, &catalog),
                            Some(Keycode::Up) => ui.direction(Direction::Up, &catalog),
                            Some(Keycode::Down) => ui.direction(Direction::Down, &catalog),
                            Some(Keycode::Return | Keycode::KpEnter) if !repeat => {
                                ui.confirm_down(ConfirmButton::Key, Instant::now(), &catalog);
                            }
                            Some(Keycode::Backspace) if ui.panel == Some(Panel::City) => {
                                ui.city.delete();
                                ui.panel_index = city::SEARCH;
                                ui.confirm = None;
                            }
                            Some(Keycode::Escape | Keycode::Backspace) if !repeat => ui.back(),
                            _ => interacted = false,
                        }
                    }
                }
                Event::TextInput { text, .. } if ui.panel == Some(Panel::City) => {
                    ui.city.type_text(&text);
                    ui.panel_index = city::SEARCH;
                    ui.confirm = None;
                    interacted = true;
                }
                Event::KeyUp {
                    keycode: Some(Keycode::Return | Keycode::KpEnter),
                    ..
                } => {
                    launch = ui.confirm_up(
                        ConfirmButton::Key,
                        Instant::now(),
                        &catalog,
                        !preference_pending,
                    );
                    interacted = true;
                }
                Event::MouseMotion { x, y, .. } => {
                    let (w, h) = window.size();
                    interacted = ui.pointer(x * 1920.0 / w as f32, y * 1080.0 / h as f32, &catalog);
                }
                Event::MouseButtonDown {
                    x,
                    y,
                    mouse_btn: sdl3::mouse::MouseButton::Left,
                    ..
                } => {
                    let (w, h) = window.size();
                    if ui.pointer(x * 1920.0 / w as f32, y * 1080.0 / h as f32, &catalog) {
                        ui.confirm_down(ConfirmButton::Pointer, Instant::now(), &catalog);
                        interacted = true;
                    }
                }
                Event::MouseButtonUp {
                    x,
                    y,
                    mouse_btn: sdl3::mouse::MouseButton::Left,
                    ..
                } => {
                    let (w, h) = window.size();
                    if ui.pointer(x * 1920.0 / w as f32, y * 1080.0 / h as f32, &catalog) {
                        launch = ui.confirm_up(
                            ConfirmButton::Pointer,
                            Instant::now(),
                            &catalog,
                            !preference_pending,
                        );
                    } else {
                        ui.confirm = None;
                    }
                    interacted = true;
                }
                Event::MouseWheel { y, .. }
                    if y != 0.0 && wheel_at.elapsed() > Duration::from_millis(100) =>
                {
                    wheel_at = Instant::now();
                    ui.direction(
                        if y > 0.0 {
                            Direction::Up
                        } else {
                            Direction::Down
                        },
                        &catalog,
                    );
                    interacted = true;
                }
                _ => (),
            }
            if ui.order_changed {
                refresh_grid(
                    &mut ui,
                    &mut catalog,
                    &prefs,
                    &source_catalog,
                    &mut renderer,
                    &source_icons,
                );
            }
            if previous_panel == Some(Panel::SpeedTest) && ui.panel != previous_panel {
                speed_test.cancel();
            }
            if launch == Some(Action::SpeedTest) {
                speed_test.toggle();
            }
            if launch == Some(Action::RefreshWeather) {
                weather_monitor.refresh();
            }
            if let Some(Action::SearchCity(generation, query)) = &launch {
                if city_worker
                    .requests
                    .try_send((*generation, query.clone()))
                    .is_err()
                {
                    ui.city
                        .finish(*generation, Err(anyhow::anyhow!("City search busy")));
                }
            }
            if ui.panel != previous_panel {
                if ui.panel == Some(Panel::City) {
                    video.text_input().start(&window);
                }
                if previous_panel == Some(Panel::City) {
                    video.text_input().stop(&window);
                }
            }
            let current_target = ui.focus_target();
            if focused
                && navigation
                && current_target.is_some()
                && current_target != previous_target
            {
                sounds.play();
            }
            if ui.panel == Some(Panel::Wifi) && previous_panel != ui.panel {
                wifi_monitor.refresh(Instant::now());
            }
            let change = match &launch {
                Some(Action::TogglePin(id)) => Some(prefs.toggled(id)),
                Some(Action::SaveOrder(order)) => Some(prefs.with_order(order)),
                Some(Action::ChooseCity(location)) => Some(prefs.with_location(location.clone())),
                _ => None,
            };
            if let Some(change) = change {
                if !preference_pending {
                    if preference_writer.requests.try_send(change).is_ok() {
                        preference_pending = true;
                        if matches!(launch, Some(Action::ChooseCity(_))) {
                            ui.city.saving = true;
                        }
                        ui.toast = None;
                        if let Some(moving) = &mut ui.moving {
                            moving.saving = true;
                        }
                    } else {
                        ui.toast = Some((
                            "Could not save changes. Reopen Home.".into(),
                            Instant::now(),
                        ));
                    }
                }
            }
            if let Some(Action::Launch(id, title)) = launch {
                let request = (id, title);
                sounds.suspend();
                ui.toast = None;
                if let Err(std::sync::mpsc::TrySendError::Disconnected(_)) =
                    launcher.requests.try_send(request)
                {
                    ui.toast = Some((
                        "App launcher unavailable. Reopen Home.".into(),
                        Instant::now(),
                    ));
                }
            }
            if interacted {
                input_at.get_or_insert_with(Instant::now);
                dirty = true;
            }
        }
        dirty |= ui.hold_tick(Instant::now(), &catalog, focused && !preference_pending);
        dirty |= speed_test.poll();
        for (generation, result) in city_worker.results.try_iter() {
            if ui.city.finish(generation, result) {
                ui.confirm = None;
                if ui.panel == Some(Panel::City)
                    && ui.panel_index == city::SEARCH
                    && !ui.city.results.is_empty()
                {
                    ui.panel_index = city::RESULTS;
                }
                dirty = true;
            }
        }
        let mut refresh_catalog = false;
        for update in catalog_worker.results.try_iter() {
            source_catalog = update.catalog;
            source_icons = update.icons;
            refresh_catalog = true;
        }
        for result in preference_writer.results.try_iter() {
            preference_pending = false;
            let city_saved = ui.city.saving;
            ui.city.saving = false;
            match result {
                Ok(saved) => {
                    prefs = saved;
                    weather_monitor.select(prefs.weather_location.clone());
                    if city_saved && ui.panel == Some(Panel::City) {
                        ui.back();
                        video.text_input().stop(&window);
                    }
                    ui.save_finished(true);
                    refresh_catalog = true;
                }
                Err(error) => {
                    ui.save_finished(false);
                    tracing::warn!(error = %format!("{error:#}"), "Could not save Home preferences");
                    ui.toast = Some((preferences::save_error(&error), Instant::now()));
                    dirty = true;
                }
            }
        }
        if refresh_catalog {
            refresh_grid(
                &mut ui,
                &mut catalog,
                &prefs,
                &source_catalog,
                &mut renderer,
                &source_icons,
            );
            dirty = true;
        }
        if focused
            && catalog_check.elapsed() >= Duration::from_secs(5)
            && catalog_worker.requests.try_send(()).is_ok()
        {
            catalog_check = Instant::now();
        }
        if platform::exit_held() {
            ui.quit = true;
        }
        for status in wifi_worker.results.try_iter() {
            dirty |= wifi_monitor.finish(status, Instant::now());
        }
        let now = Instant::now();
        dirty |= wifi_monitor.expire(now);
        dirty |= weather_monitor.tick(focused);
        if wifi_monitor.due(now, focused) {
            match wifi_worker.requests.try_send(()) {
                Ok(()) => wifi_monitor.start(),
                Err(_) => {
                    dirty |= wifi_monitor.finish(wifi::Status::default(), now);
                }
            }
        }
        for response in launcher.results.try_iter() {
            if let Err(error) = response {
                tracing::warn!(error = %error, "launch failed");
                ui.toast = Some((error, Instant::now()));
                dirty = true;
            }
        }
        if ui
            .toast
            .as_ref()
            .is_some_and(|(_, at)| at.elapsed() > Duration::from_secs(4))
        {
            ui.toast = None;
            dirty = true;
        }
        for (i, scale) in ui.scales.iter_mut().enumerate() {
            scale.set(
                if i == ui.selected && ui.header.is_none() && ui.panel.is_none() {
                    1.14
                } else {
                    1.0
                },
            );
        }
        if clock_check.elapsed() > Duration::from_secs(1) {
            clock_check = Instant::now();
            let updated = platform::time_label();
            if updated != clock {
                clock = updated;
                date = platform::clock(c"%A, %e %B");
                dirty = true;
            }
        }
        let animating = ui.moving.is_some()
            || ui.scroll.active()
            || ui.scales.iter().any(Tween::active)
            || ui.positions.iter().any(TileMotion::active);
        if focused && (dirty || animating) {
            let draw_start = Instant::now();
            graphics.make_current(&window)?;
            let size = window.size();
            let items = ui.items(&catalog);
            let input_pins: Vec<bool> = catalog
                .input_items()
                .iter()
                .map(|i| prefs.contains(&i.app_id))
                .collect();
            let scales: Vec<f32> = ui.scales.iter().map(Tween::value).collect();
            let positions: Vec<_> = ui
                .positions
                .iter()
                .map(|p| p.value_at(draw_start))
                .collect();
            renderer.draw(
                graphics.surface(size.0, size.1)?.canvas(),
                &render::Frame {
                    catalog: &catalog,
                    selected: ui.selected,
                    status: ui.header == Some(Header::Controls),
                    scroll: ui.scroll.value(),
                    scales: &scales,
                    positions: &positions,
                    clock: &clock,
                    date: &date,
                    panel: ui
                        .panel
                        .filter(|p| {
                            !matches!(
                                p,
                                Panel::Wifi | Panel::SpeedTest | Panel::Weather | Panel::City
                            )
                        })
                        .map(|_| (items.as_slice(), ui.panel_index)),
                    input_pins: (ui.panel == Some(Panel::Inputs)).then_some(input_pins.as_slice()),
                    pin_focused: ui.pin_focused,
                    wifi: &wifi_monitor.status,
                    wifi_focused: ui.header == Some(Header::Wifi),
                    wifi_details: ui.panel == Some(Panel::Wifi),
                    wifi_selection: ui.panel_index,
                    speed_test: (ui.panel == Some(Panel::SpeedTest))
                        .then_some((&speed_test.state, ui.panel_index)),
                    weather: &weather_monitor.state,
                    weather_focused: ui.header == Some(Header::Weather),
                    weather_details: ui.panel == Some(Panel::Weather),
                    weather_selection: ui.panel_index,
                    weather_location: prefs.weather_location.as_ref(),
                    city_picker: (ui.panel == Some(Panel::City))
                        .then_some((&ui.city, ui.panel_index)),
                    move_elapsed: ui
                        .moving
                        .as_ref()
                        .map(|m| m.started.elapsed().as_secs_f32()),
                    move_hint: ui.moving.as_ref().map(|m| {
                        if m.saving {
                            "Saving order…"
                        } else {
                            "Move with arrows · OK to save · Back to cancel"
                        }
                    }),
                    toast: ui.toast.as_ref().map(|(s, _)| s.as_str()),
                    metrics: if ui.metrics { Some(&metric_text) } else { None },
                },
                size,
            );
            graphics.flush();
            let submit_ms = draw_start.elapsed().as_secs_f64() * 1000.0;
            let input_ms = input_at
                .take()
                .map(|at| at.elapsed().as_secs_f64() * 1000.0);
            window.gl_swap_window();
            let present_ms = draw_start.elapsed().as_secs_f64() * 1000.0;
            draw_samples.push(present_ms);
            frame_count += 1;
            if let Some(ms) = input_ms {
                tracing::info!(
                    input_submit_ms = format_args!("{ms:.2}"),
                    submit_ms = format_args!("{submit_ms:.2}"),
                    present_ms = format_args!("{present_ms:.2}"),
                    "input frame"
                );
            }
            if last_metrics.elapsed() > Duration::from_secs(2) {
                draw_samples.sort_by(f64::total_cmp);
                let p95 = draw_samples[(draw_samples.len() * 95 / 100).min(draw_samples.len() - 1)];
                metric_text = format!("Native · draw + swap p95 {p95:.1} ms · {frame_count} frames · F8 / Blue to hide");
                draw_samples.clear();
                last_metrics = Instant::now();
            }
            // One final frame after the animation deadline paints the exact target position.
            dirty = animating;
        }
        let delay = if focused && (dirty || animating) {
            Duration::from_micros(16_667).saturating_sub(frame_start.elapsed())
        } else {
            Duration::from_millis(100)
        };
        if delay.as_millis() > 0 {
            // NULL leaves the event queued. New input wakes this immediately, including while idle.
            unsafe {
                sdl3::sys::events::SDL_WaitEventTimeout(
                    std::ptr::null_mut(),
                    delay.as_millis() as i32,
                );
            }
        }
    }
    speed_test.cancel();
    graphics.release_resources();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn city_picker_remote_keyboard_results_pointer_and_cancel() {
        let catalog = catalog();
        let mut ui = Ui::new(catalog.apps.len());
        ui.panel = Some(Panel::Weather);
        ui.activate(&catalog);
        assert_eq!(ui.panel, Some(Panel::City));
        ui.activate(&catalog); // Q
        ui.direction(Direction::Right, &catalog);
        ui.activate(&catalog); // W
        assert_eq!(ui.city.query, "QW");
        ui.panel_index = city::DELETE;
        ui.activate(&catalog);
        assert_eq!(ui.city.query, "Q");
        ui.panel_index = city::CLEAR;
        ui.activate(&catalog);
        ui.city.type_text("Mumbai");
        ui.panel_index = city::SEARCH;
        assert!(
            matches!(ui.activate(&catalog), Some(Action::SearchCity(_, query)) if query == "Mumbai")
        );
        let place = weather::Location {
            city: "Mumbai".into(),
            latitude: 19.07,
            longitude: 72.88,
            region: "Maharashtra".into(),
            country: "India".into(),
        };
        ui.city.results = vec![place.clone()];
        let (x, y, _, _) = city::bounds(city::RESULTS);
        assert!(ui.pointer(x + 20.0, y + 20.0, &catalog));
        assert_eq!(ui.activate(&catalog), Some(Action::ChooseCity(Some(place))));
        ui.city.saving = true;
        assert!(ui.activate(&catalog).is_none());
        ui.city.saving = false;
        ui.panel_index = city::AUTO;
        assert_eq!(ui.activate(&catalog), Some(Action::ChooseCity(None)));
        ui.back();
        assert_eq!(ui.panel, Some(Panel::Weather));
        assert!(!ui.city.searching);
    }

    #[test]
    fn weather_navigation_refresh_back_and_pointer() {
        let catalog = catalog();
        let mut ui = Ui::new(catalog.apps.len());
        ui.direction(Direction::Up, &catalog);
        assert_eq!(ui.header, Some(Header::Weather));
        assert_eq!(ui.activate(&catalog), None);
        assert_eq!(ui.panel, Some(Panel::Weather));
        ui.direction(Direction::Down, &catalog);
        assert_eq!(ui.activate(&catalog), Some(Action::RefreshWeather));
        ui.direction(Direction::Down, &catalog);
        ui.activate(&catalog);
        assert_eq!(ui.panel, None);
        assert_eq!(ui.header, Some(Header::Weather));
        ui.direction(Direction::Right, &catalog);
        assert_eq!(ui.header, Some(Header::Controls));
        ui.direction(Direction::Down, &catalog);
        assert_eq!(ui.header, None);
        let (x, y, _, _) = weather_bounds(catalog.apps.len());
        assert!(ui.pointer(x + 30.0, y + 30.0, &catalog));
        ui.activate(&catalog);
        assert!(!ui.pointer(1700.0, 800.0, &catalog));
        let (x, y, _, _) = weather_action_bounds(2);
        assert!(ui.pointer(x + 20.0, y + 20.0, &catalog));
        ui.activate(&catalog);
        ui.back();
        assert!(!ui.quit);
        for count in [1, 6, 7, 12, 13, 18, 30] {
            let (_, y, _, h) = weather_bounds(count);
            assert!(y + h < grid_top(count) - PAD);
        }
    }

    fn pick_up(ui: &mut Ui, catalog: &Catalog, button: ConfirmButton) {
        let now = Instant::now();
        ui.confirm_down(button, now, catalog);
        assert!(!ui.hold_tick(now + MOVE_HOLD - Duration::from_millis(1), catalog, true));
        // Duplicate down events must not reset the long-press timer.
        ui.confirm_down(button, now + Duration::from_millis(300), catalog);
        assert!(ui.hold_tick(now + MOVE_HOLD, catalog, true));
        assert_eq!(ui.confirm_up(button, now + MOVE_HOLD, catalog, true), None);
        assert!(ui.moving.is_some());
    }

    #[test]
    fn taps_launch_once_and_holds_enter_move_mode_without_launching() {
        let catalog = catalog();
        for button in [ConfirmButton::Key, ConfirmButton::Pointer] {
            let mut ui = Ui::new(catalog.apps.len());
            let now = Instant::now();
            ui.confirm_down(button, now, &catalog);
            assert_eq!(
                ui.confirm_up(button, now + Duration::from_millis(80), &catalog, true),
                Some(Action::Launch("preview.0".into(), "0".into()))
            );
            assert_eq!(ui.confirm_up(button, now, &catalog, true), None);
            pick_up(&mut ui, &catalog, button);
            ui.confirm_down(button, now, &catalog);
            assert_eq!(
                ui.confirm_up(button, now, &catalog, true),
                Some(Action::SaveOrder(preferences::tile_ids(&catalog)))
            );
        }
        let mut ui = Ui::new(catalog.apps.len());
        let now = Instant::now();
        ui.confirm_down(ConfirmButton::Key, now, &catalog);
        // A release processed before an idle tick still detects a hold.
        assert_eq!(
            ui.confirm_up(ConfirmButton::Key, now + MOVE_HOLD, &catalog, true),
            None
        );
        assert!(ui.moving.is_some());
    }

    #[test]
    fn moving_hdmi_across_rows_can_save_cancel_and_retry_failed_writes() {
        let mut source = catalog();
        for i in 2..8 {
            let mut app = source.apps[0].clone();
            app.id = format!("preview.{i}");
            source.apps.push(app);
        }
        let prefs = preferences::Preferences::default().toggled("preview.hdmi1");
        let mut display = prefs.catalog(&source);
        let mut ui = Ui::new(display.apps.len());
        ui.select(8, display.apps.len());
        pick_up(&mut ui, &display, ConfirmButton::Key);
        ui.direction(Direction::Up, &display);
        ui.refresh_catalog(&mut display, &prefs, &source);
        assert_eq!(ui.selected, 2);
        assert!(display.apps[2].input);
        assert_eq!(display.apps[3].id, "preview.2");
        ui.direction(Direction::Up, &display); // Top boundary stays in move mode.
        assert!(ui.header.is_none());
        assert_eq!(ui.selected, 2);
        ui.back();
        ui.refresh_catalog(&mut display, &prefs, &source);
        assert_eq!(ui.selected, 8);
        assert_eq!(
            preferences::tile_ids(&display),
            preferences::tile_ids(&prefs.catalog(&source))
        );
        assert!(!ui.quit);
        pick_up(&mut ui, &display, ConfirmButton::Key);
        ui.direction(Direction::Up, &display);
        ui.refresh_catalog(&mut display, &prefs, &source);
        let Some(Action::SaveOrder(order)) = ui.activate(&display) else {
            panic!("save order expected");
        };
        ui.moving.as_mut().unwrap().saving = true;
        ui.direction(Direction::Right, &display);
        ui.back();
        assert_eq!(ui.selected, 2);
        assert!(ui.moving.is_some());
        ui.save_finished(false);
        assert_eq!(
            ui.activate(&display),
            Some(Action::SaveOrder(order.clone()))
        );
        let saved = prefs.with_order(&order);
        ui.save_finished(true);
        ui.refresh_catalog(&mut display, &saved, &source);
        assert!(ui.moving.is_none());
        assert_eq!(ui.selected, 2);
        assert_eq!(
            ui.activate(&display),
            Some(Action::Launch("preview.hdmi1".into(), "HDMI 1".into()))
        );
    }

    #[test]
    fn tile_positions_follow_ids_during_horizontal_vertical_and_interrupted_moves() {
        let mut source = catalog();
        for i in 2..10 {
            let mut app = source.apps[0].clone();
            app.id = format!("preview.{i}");
            source.apps.push(app);
        }
        let prefs = preferences::Preferences {
            pinned_inputs: vec!["preview.hdmi1".into()],
            tile_order: vec!["preview.hdmi1".into()],
            ..Default::default()
        };
        let mut display = prefs.catalog(&source);
        let mut ui = Ui::new(display.apps.len());
        pick_up(&mut ui, &display, ConfirmButton::Key);
        // Issue each next direction before the previous slide has finished.
        for direction in [
            Direction::Right,
            Direction::Down,
            Direction::Left,
            Direction::Up,
        ] {
            let previous = display.clone();
            let motions = ui.positions.clone();
            ui.direction(direction, &display);
            ui.refresh_catalog(&mut display, &prefs, &source);
            assert_eq!(display.apps[ui.selected].id, "preview.hdmi1");
            for (i, app) in display.apps.iter().enumerate() {
                let old = previous.apps.iter().position(|a| a.id == app.id).unwrap();
                let slide = ui.positions[i];
                assert_eq!(slide.to, position(i, display.apps.len()));
                if motions[old].to != slide.to {
                    assert_eq!(slide.from, motions[old].value_at(slide.start));
                } else {
                    assert_eq!(slide.start, motions[old].start);
                    assert_eq!(slide.from, motions[old].from);
                }
            }
        }
        // A refresh that keeps the visible order must not restart a slide.
        let starts: Vec<_> = ui.positions.iter().map(|m| m.start).collect();
        source.apps.reverse();
        ui.refresh_catalog(&mut display, &prefs, &source);
        assert_eq!(
            starts,
            ui.positions.iter().map(|m| m.start).collect::<Vec<_>>()
        );
        // Cancellation retargets all tiles to saved positions using the current visual positions.
        ui.direction(Direction::Down, &display);
        ui.refresh_catalog(&mut display, &prefs, &source);
        let previous = display.clone();
        let motions = ui.positions.clone();
        ui.back();
        ui.refresh_catalog(&mut display, &prefs, &source);
        assert!(ui.moving.is_none());
        assert_eq!(
            preferences::tile_ids(&display),
            preferences::tile_ids(&prefs.catalog(&source))
        );
        for (i, app) in display.apps.iter().enumerate() {
            let old = previous.apps.iter().position(|a| a.id == app.id).unwrap();
            let slide = ui.positions[i];
            assert_eq!(slide.to, position(i, display.apps.len()));
            if motions[old].to != slide.to {
                assert_eq!(slide.from, motions[old].value_at(slide.start));
            }
        }
    }

    #[test]
    fn catalog_updates_during_a_move_keep_identity_and_cancel_if_tile_disappears() {
        let mut source = catalog();
        let prefs = preferences::Preferences::default();
        let mut display = prefs.catalog(&source);
        let mut ui = Ui::new(display.apps.len());
        pick_up(&mut ui, &display, ConfirmButton::Key);
        ui.direction(Direction::Right, &display);
        ui.refresh_catalog(&mut display, &prefs, &source);
        source.apps.reverse();
        let mut new = source.apps[0].clone();
        new.id = "new".into();
        source.apps.insert(0, new);
        ui.refresh_catalog(&mut display, &prefs, &source);
        assert_eq!(display.apps[ui.selected].id, "preview.0");
        assert_eq!(display.apps.last().unwrap().id, "new");
        source.apps.retain(|app| app.id != "preview.0");
        ui.refresh_catalog(&mut display, &prefs, &source);
        assert!(ui.moving.is_none());
        assert_eq!(
            preferences::tile_ids(&display),
            preferences::tile_ids(&source)
        );
    }

    #[test]
    fn changed_targets_focus_loss_and_pointer_jitter_do_not_launch_or_move_the_wrong_tile() {
        let source = catalog();
        let now = Instant::now();
        let mut ui = Ui::new(source.apps.len());
        ui.confirm_down(ConfirmButton::Pointer, now, &source);
        ui.select(1, source.apps.len());
        assert_eq!(
            ui.confirm_up(ConfirmButton::Pointer, now, &source, true),
            None
        );
        ui.confirm_down(ConfirmButton::Key, now, &source);
        ui.cancel_move(); // Focus loss clears pending clicks, even outside move mode.
        assert_eq!(
            ui.confirm_up(ConfirmButton::Key, now + MOVE_HOLD, &source, true),
            None
        );
        pick_up(&mut ui, &source, ConfirmButton::Pointer);
        let (x, y) = position(0, source.apps.len());
        assert!(!ui.pointer(x + 10.0, y + 10.0, &source));
        assert_eq!(ui.selected, 1);
        ui.cancel_move();
        assert!(ui.moving.is_none());
        assert!(!ui.quit);
    }

    #[test]
    fn a_hold_during_a_pending_save_never_falls_through_to_launch() {
        let catalog = catalog();
        let mut ui = Ui::new(catalog.apps.len());
        let now = Instant::now();
        ui.confirm_down(ConfirmButton::Key, now, &catalog);
        assert!(!ui.hold_tick(now + MOVE_HOLD, &catalog, false));
        assert_eq!(
            ui.confirm_up(ConfirmButton::Key, now + MOVE_HOLD, &catalog, false),
            None
        );
        assert!(ui.moving.is_none());
    }

    fn catalog() -> Catalog {
        Catalog {
            preview: true,
            apps: (0..2)
                .map(|i| App {
                    id: format!("preview.{i}"),
                    title: i.to_string(),
                    artwork: String::new(),
                    contain: false,
                    color: [0, 0, 0],
                    background: None,
                    input: false,
                })
                .collect(),
            inputs: (1..=2)
                .map(|i| Input {
                    app_id: format!("preview.hdmi{i}"),
                    label: format!("HDMI {i}"),
                })
                .collect(),
        }
    }

    #[test]
    fn catalog_changes_preserve_selection_and_clamp_removed_apps_and_inputs() {
        let old = catalog();
        let mut ui = Ui::new(2);
        ui.select(1, 2);
        let mut updated = catalog();
        updated.apps.reverse();
        ui.reconcile(&old, &updated);
        assert_eq!(updated.apps[ui.selected].id, old.apps[1].id);
        ui.panel = Some(Panel::Inputs);
        ui.panel_index = 2;
        let mut reduced = catalog();
        reduced.apps.truncate(1);
        reduced.inputs.clear();
        ui.reconcile(&updated, &reduced);
        assert_eq!(ui.selected, 0);
        assert_eq!(ui.scales.len(), 1);
        assert_eq!(ui.panel_index, 0);
    }

    #[test]
    fn app_focus_ignores_pointer_jitter_and_grid_edges() {
        let catalog = catalog();
        let mut ui = Ui::new(2);
        let (x, y) = position(0, 2);
        for offset in [1.0, 20.0, 100.0] {
            assert!(ui.pointer(x + offset, y + 20.0, &catalog));
            assert_eq!(ui.focus_target(), Some(FocusTarget::App(0)));
        }
        ui.direction(Direction::Left, &catalog);
        assert_eq!(ui.focus_target(), Some(FocusTarget::App(0)));
        ui.direction(Direction::Right, &catalog);
        assert_eq!(ui.focus_target(), Some(FocusTarget::App(1)));
        ui.direction(Direction::Right, &catalog);
        assert_eq!(ui.focus_target(), Some(FocusTarget::App(1)));
        ui.direction(Direction::Up, &catalog);
        assert_eq!(
            ui.focus_target(),
            Some(FocusTarget::Header(Header::Weather))
        );
        ui.direction(Direction::Down, &catalog);
        assert_eq!(ui.focus_target(), Some(FocusTarget::App(1)));
    }

    #[test]
    fn menu_focus_tracks_remote_pointer_and_submenu_without_repeating_at_edges() {
        let catalog = catalog();
        let mut ui = Ui::new(2);
        ui.direction(Direction::Up, &catalog);
        ui.direction(Direction::Right, &catalog);
        ui.activate(&catalog);
        let first = Some(FocusTarget::Menu(Panel::Controls, 0));
        assert_eq!(ui.focus_target(), first);
        ui.direction(Direction::Up, &catalog);
        assert_eq!(ui.focus_target(), first);
        ui.direction(Direction::Right, &catalog);
        assert_eq!(ui.focus_target(), first);
        ui.direction(Direction::Down, &catalog);
        assert_eq!(
            ui.focus_target(),
            Some(FocusTarget::Menu(Panel::Controls, 1))
        );
        ui.direction(Direction::Down, &catalog);
        ui.direction(Direction::Down, &catalog);
        let last = Some(FocusTarget::Menu(Panel::Controls, 2));
        assert_eq!(ui.focus_target(), last);
        ui.direction(Direction::Down, &catalog);
        assert_eq!(ui.focus_target(), last);
        for x in [1272.0, 1300.0, 1500.0] {
            assert!(ui.pointer(x, PANEL_ROW_TOP + 20.0, &catalog));
            assert_eq!(ui.focus_target(), first);
        }
        assert!(!ui.pointer(1200.0, PANEL_ROW_TOP + PANEL_ROW_STEP, &catalog));
        assert_eq!(ui.focus_target(), first);
        ui.activate(&catalog);
        assert_eq!(ui.focus_target(), Some(FocusTarget::Menu(Panel::Inputs, 0)));
        assert_ne!(ui.focus_target(), first);
        ui.direction(Direction::Down, &catalog);
        assert_eq!(ui.focus_target(), Some(FocusTarget::Menu(Panel::Inputs, 1)));
        ui.back();
        assert_eq!(ui.focus_target(), first);
    }

    #[test]
    fn input_pin_is_separate_from_switching_with_remote_and_pointer() {
        let catalog = catalog();
        let mut ui = Ui::new(catalog.apps.len());
        ui.panel = Some(Panel::Inputs);
        ui.direction(Direction::Right, &catalog);
        assert_eq!(ui.focus_target(), Some(FocusTarget::Pin(0)));
        assert_eq!(
            ui.activate(&catalog),
            Some(Action::TogglePin("preview.hdmi1".into()))
        );
        assert_eq!(ui.panel, Some(Panel::Inputs));
        ui.direction(Direction::Down, &catalog);
        assert_eq!(ui.focus_target(), Some(FocusTarget::Pin(1)));
        ui.direction(Direction::Left, &catalog);
        assert_eq!(ui.focus_target(), Some(FocusTarget::Menu(Panel::Inputs, 1)));
        assert_eq!(
            ui.activate(&catalog),
            Some(Action::Launch("preview.hdmi2".into(), "HDMI 2".into()))
        );
        assert_eq!(ui.panel, None);
        ui.panel = Some(Panel::Inputs);
        assert!(ui.pointer(PIN_LEFT + 35.0, PANEL_ROW_TOP + 35.0, &catalog));
        assert_eq!(
            ui.activate(&catalog),
            Some(Action::TogglePin("preview.hdmi1".into()))
        );
        assert!(!ui.pointer(PIN_LEFT - 6.0, PANEL_ROW_TOP + 35.0, &catalog));
        assert!(!ui.pointer(PIN_LEFT + 35.0, PANEL_ROW_TOP + 80.0, &catalog));
        assert!(ui.pointer(1300.0, PANEL_ROW_TOP + 35.0, &catalog));
        assert_eq!(
            ui.activate(&catalog),
            Some(Action::Launch("preview.hdmi1".into(), "HDMI 1".into()))
        );
        ui.panel = Some(Panel::Inputs);
        ui.pin_focused = true;
        ui.back();
        assert_eq!(ui.panel, Some(Panel::Controls));
        assert!(!ui.pin_focused);
    }

    #[test]
    fn pinned_tile_launches_input_and_catalog_refresh_keeps_its_selection() {
        let base = catalog();
        let pins = preferences::Preferences::default().toggled("preview.hdmi1");
        let catalog = pins.catalog(&base);
        let mut ui = Ui::new(catalog.apps.len());
        ui.select(2, catalog.apps.len());
        assert_eq!(
            ui.activate(&catalog),
            Some(Action::Launch("preview.hdmi1".into(), "HDMI 1".into()))
        );
        let mut refreshed = base.clone();
        refreshed.apps.reverse();
        let refreshed = pins.catalog(&refreshed);
        ui.reconcile(&catalog, &refreshed);
        assert_eq!(refreshed.apps[ui.selected].id, "preview.hdmi1");
        ui.reconcile(&refreshed, &base);
        assert!(ui.selected < base.apps.len());
        assert_eq!(ui.scales.len(), base.apps.len());
    }

    #[test]
    fn wifi_icon_opens_details_and_returns_to_the_icon() {
        let catalog = catalog();
        let mut ui = Ui::new(2);
        ui.direction(Direction::Up, &catalog);
        ui.direction(Direction::Right, &catalog);
        assert_eq!(
            ui.focus_target(),
            Some(FocusTarget::Header(Header::Controls))
        );
        ui.direction(Direction::Left, &catalog);
        assert_eq!(
            ui.focus_target(),
            Some(FocusTarget::Header(Header::Weather))
        );
        ui.direction(Direction::Right, &catalog);
        assert_eq!(
            ui.focus_target(),
            Some(FocusTarget::Header(Header::Controls))
        );
        ui.direction(Direction::Right, &catalog);
        assert_eq!(ui.focus_target(), Some(FocusTarget::Header(Header::Wifi)));
        ui.direction(Direction::Right, &catalog);
        assert!(ui.activate(&catalog).is_none());
        assert_eq!(ui.panel, Some(Panel::Wifi));
        assert!(!ui.pointer(200.0, 400.0, &catalog));
        let (x, y, _, _) = wifi_action_bounds(1);
        assert!(ui.pointer(x + 20.0, y + 20.0, &catalog));
        ui.activate(&catalog);
        assert_eq!(ui.panel, None);
        assert_eq!(ui.header, Some(Header::Wifi));
        ui.back();
        assert_eq!(ui.header, None);
        assert!(!ui.quit);
        let (x, y, _, _) = wifi_bounds();
        assert!(ui.pointer(x + 30.0, y + 30.0, &catalog));
        ui.activate(&catalog);
        ui.back();
        assert_eq!(ui.header, Some(Header::Wifi));
        ui.direction(Direction::Left, &catalog);
        ui.activate(&catalog);
        assert_eq!(ui.items(&catalog), ["Inputs", "TV settings", "Exit"]);
        ui.direction(Direction::Down, &catalog);
        ui.direction(Direction::Down, &catalog);
        ui.activate(&catalog);
        assert!(ui.quit);
    }

    #[test]
    fn speed_test_requires_explicit_start_and_back_returns_to_wifi() {
        let catalog = catalog();
        let mut ui = Ui::new(2);
        ui.direction(Direction::Up, &catalog);
        ui.direction(Direction::Right, &catalog);
        ui.direction(Direction::Right, &catalog);
        assert!(ui.activate(&catalog).is_none());
        assert_eq!(ui.panel, Some(Panel::Wifi));
        assert!(ui.activate(&catalog).is_none());
        assert_eq!(ui.panel, Some(Panel::SpeedTest));
        assert_eq!(ui.activate(&catalog), Some(Action::SpeedTest));
        let (x, y, _, _) = speed_action_bounds(1);
        assert!(!ui.pointer(x, y - 5.0, &catalog));
        assert!(ui.pointer(x + 20.0, y + 20.0, &catalog));
        assert!(ui.activate(&catalog).is_none());
        assert_eq!(ui.panel, Some(Panel::Wifi));
        assert_eq!(ui.panel_index, 0);
        ui.activate(&catalog);
        ui.direction(Direction::Left, &catalog);
        assert_eq!(ui.panel, Some(Panel::Wifi));
    }
}
