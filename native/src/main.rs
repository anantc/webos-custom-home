mod audio;
mod catalog;
mod gl;
mod model;
mod platform;
mod preferences;
mod render;
mod speedtest;

use anyhow::{Context, Result};
use model::*;
use platform::wifi;
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
}
#[derive(Debug, PartialEq)]
enum Action {
    Launch(String, String),
    TogglePin(String),
    SpeedTest,
}
struct Ui {
    selected: usize,
    header: Option<Header>,
    panel: Option<Panel>,
    panel_index: usize,
    pin_focused: bool,
    scroll: Tween,
    scales: Vec<Tween>,
    toast: Option<(String, Instant)>,
    metrics: bool,
    quit: bool,
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
            toast: None,
            metrics: false,
            quit: false,
        }
    }
    fn reconcile(&mut self, previous: &Catalog, updated: &Catalog) {
        let id = previous.apps.get(self.selected).map(|app| app.id.as_str());
        self.selected = updated
            .apps
            .iter()
            .position(|app| Some(app.id.as_str()) == id)
            .unwrap_or_else(|| self.selected.min(updated.apps.len() - 1));
        self.scales = (0..updated.apps.len())
            .map(|i| {
                Tween::new(
                    if i == self.selected && self.header.is_none() && self.panel.is_none() {
                        1.14
                    } else {
                        1.0
                    },
                )
            })
            .collect();
        self.scroll = Tween::new(scroll_for(
            self.selected,
            updated.apps.len(),
            self.scroll.to,
        ));
        self.panel_index = self
            .panel_index
            .min(self.items(updated).len().saturating_sub(1));
    }
    fn items(&self, catalog: &Catalog) -> Vec<String> {
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
                Direction::Left if header == Header::Controls => self.header = Some(Header::Wifi),
                Direction::Right if header == Header::Wifi => self.header = Some(Header::Controls),
                _ => (),
            }
        } else if let Some(next) = next(self.selected, direction, catalog.apps.len()) {
            self.select(next, catalog.apps.len());
        } else {
            self.header = Some(if self.selected % COLS < COLS / 2 {
                Header::Wifi
            } else {
                Header::Controls
            });
        }
    }
    fn back(&mut self) {
        self.pin_focused = false;
        if self.panel == Some(Panel::SpeedTest) {
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
            });
            self.panel_index = 0;
        } else {
            let app = &catalog.apps[self.selected];
            return Some(Action::Launch(app.id.clone(), app.title.clone()));
        }
        None
    }
    fn pointer(&mut self, x: f32, y: f32, catalog: &Catalog) -> bool {
        if matches!(self.panel, Some(Panel::Wifi | Panel::SpeedTest)) {
            for i in 0..2 {
                let (left, top, width, height) = if self.panel == Some(Panel::Wifi) {
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
            if (1244.0..1796.0).contains(&x) && y >= PANEL_ROW_TOP {
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
        ] {
            if (left..left + width).contains(&x) && (top..top + height).contains(&y) {
                self.header = Some(header);
                return true;
            }
        }
        for i in 0..catalog.apps.len() {
            let (cx, cy) = position(i, catalog.apps.len());
            let cy = cy - self.scroll.value();
            if (cx..cx + CARD_W).contains(&x) && (cy..cy + CARD_H).contains(&y) {
                self.select(i, catalog.apps.len());
                return true;
            }
        }
        false
    }
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
    let mut pins = preferences::Pins::load(&preferences_path).unwrap_or_else(|error| {
        tracing::warn!(%error, "Could not read pinned inputs");
        preferences::Pins::default()
    });
    let result_path = preferences_path.with_file_name(if preview {
        "preview-speedtest-result.json"
    } else {
        "speedtest-result.json"
    });
    let mut speed_test = speedtest::Monitor::new(result_path, preview);
    let pin_writer = preferences::writer(preferences_path);
    let mut pin_pending = false;
    let mut catalog = pins.catalog(&source_catalog);
    let catalog_worker = catalog::worker(assets.clone(), preview, initial_catalog);
    let mut catalog_check = Instant::now();
    let mut renderer = render::Renderer::new(&assets, catalog.apps.len())?;
    let decoded = render::decode_icons(&assets, &source_catalog.apps);
    let launcher = platform::luna::worker(preview);
    let wifi_worker = wifi::worker(preview);
    let mut wifi_monitor = wifi::Monitor::new(Instant::now());
    // Set before window creation; leave Home owned by webOS so it remains an escape route.
    sdl3::hint::set("SDL_WEBOS_ACCESS_POLICY_KEYS_BACK", "true");
    sdl3::hint::set("SDL_WEBOS_ACCESS_POLICY_KEYS_EXIT", "true");
    let sdl = sdl3::init().map_err(anyhow::Error::msg)?;
    let video = sdl.video().map_err(anyhow::Error::msg)?;
    video.gl_attr().set_alpha_size(8);
    video.gl_attr().set_stencil_size(8);
    let window = video
        .window("Home", 1920, 1080)
        .opengl()
        .fullscreen()
        .build()?;
    // SDL inhibits idle screensavers by default; leave idle policy to webOS.
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
    let warm_items = ui.items(&catalog);
    let scales: Vec<f32> = ui.scales.iter().map(Tween::value).collect();
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
                                launch = ui.activate(&catalog)
                            }
                            Some(Keycode::Escape | Keycode::Backspace) if !repeat => ui.back(),
                            _ => interacted = false,
                        }
                    }
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
                        launch = ui.activate(&catalog);
                        interacted = true;
                    }
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
            if previous_panel == Some(Panel::SpeedTest) && ui.panel != previous_panel {
                speed_test.cancel();
            }
            if launch == Some(Action::SpeedTest) {
                speed_test.toggle();
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
            if let Some(Action::TogglePin(id)) = &launch {
                if !pin_pending {
                    if pin_writer.requests.try_send(pins.toggled(id)).is_ok() {
                        pin_pending = true;
                    } else {
                        ui.toast = Some((
                            "Could not save pinned inputs. Reopen Home.".into(),
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
        dirty |= speed_test.poll();
        let mut refresh_catalog = false;
        for update in catalog_worker.results.try_iter() {
            source_catalog = update.catalog;
            source_icons = update.icons;
            refresh_catalog = true;
        }
        for result in pin_writer.results.try_iter() {
            pin_pending = false;
            match result {
                Ok(saved) => {
                    pins = saved;
                    refresh_catalog = true;
                }
                Err(error) => {
                    tracing::warn!(error = %format!("{error:#}"), "Could not save pinned inputs");
                    ui.toast = Some((preferences::save_error(&error), Instant::now()));
                    dirty = true;
                }
            }
        }
        if refresh_catalog {
            let updated = pins.catalog(&source_catalog);
            ui.reconcile(&catalog, &updated);
            catalog = updated;
            renderer.icons = source_icons.clone();
            renderer.icons.resize(catalog.apps.len(), None);
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
        let animating = ui.scroll.active() || ui.scales.iter().any(Tween::active);
        if focused && (dirty || animating) {
            let draw_start = Instant::now();
            graphics.make_current(&window)?;
            let size = window.size();
            let items = ui.items(&catalog);
            let input_pins: Vec<bool> = catalog
                .input_items()
                .iter()
                .map(|i| pins.contains(&i.app_id))
                .collect();
            let scales: Vec<f32> = ui.scales.iter().map(Tween::value).collect();
            renderer.draw(
                graphics.surface(size.0, size.1)?.canvas(),
                &render::Frame {
                    catalog: &catalog,
                    selected: ui.selected,
                    status: ui.header == Some(Header::Controls),
                    scroll: ui.scroll.value(),
                    scales: &scales,
                    clock: &clock,
                    date: &date,
                    panel: ui
                        .panel
                        .filter(|p| !matches!(p, Panel::Wifi | Panel::SpeedTest))
                        .map(|_| (items.as_slice(), ui.panel_index)),
                    input_pins: (ui.panel == Some(Panel::Inputs)).then_some(input_pins.as_slice()),
                    pin_focused: ui.pin_focused,
                    wifi: &wifi_monitor.status,
                    wifi_focused: ui.header == Some(Header::Wifi),
                    wifi_details: ui.panel == Some(Panel::Wifi),
                    wifi_selection: ui.panel_index,
                    speed_test: (ui.panel == Some(Panel::SpeedTest))
                        .then_some((&speed_test.state, ui.panel_index)),
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
        assert_eq!(ui.focus_target(), Some(FocusTarget::Header(Header::Wifi)));
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
        let pins = preferences::Pins::default().toggled("preview.hdmi1");
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
        assert_eq!(ui.focus_target(), Some(FocusTarget::Header(Header::Wifi)));
        ui.direction(Direction::Left, &catalog);
        assert_eq!(ui.focus_target(), Some(FocusTarget::Header(Header::Wifi)));
        ui.direction(Direction::Right, &catalog);
        assert_eq!(
            ui.focus_target(),
            Some(FocusTarget::Header(Header::Controls))
        );
        ui.direction(Direction::Left, &catalog);
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
        ui.direction(Direction::Right, &catalog);
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
