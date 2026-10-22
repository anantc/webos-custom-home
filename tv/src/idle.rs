use sdl3::event::Event;
use std::time::{Duration, Instant};

const IDLE_DELAY: Duration = Duration::from_secs(5 * 60);
const FADE: Duration = Duration::from_millis(600);
const DIM_ALPHA: f32 = 0.7;

pub struct Dimmer {
    last_input: Instant,
    delay: Duration,
    visible: bool,
    alpha: f32,
}

impl Dimmer {
    pub fn new(now: Instant, preview: bool) -> Self {
        let delay = if preview {
            std::env::var("TVHOME_PREVIEW_DIM_SECONDS")
                .ok()
                .and_then(|s| s.parse::<u64>().ok())
                .filter(|seconds| *seconds > 0)
                .map(Duration::from_secs)
                .unwrap_or(IDLE_DELAY)
        } else {
            IDLE_DELAY
        };
        Self {
            last_input: now,
            delay,
            visible: true,
            alpha: 0.0,
        }
    }

    pub fn input(&mut self, event: &Event, now: Instant) -> bool {
        if !self.visible || crate::platform::cursor::is_notification(event) {
            return false;
        }
        let activity = match event {
            Event::KeyDown { .. }
            | Event::KeyUp { .. }
            | Event::MouseButtonDown { .. }
            | Event::MouseButtonUp { .. } => true,
            Event::MouseMotion { xrel, yrel, .. } => *xrel != 0.0 || *yrel != 0.0,
            Event::MouseWheel { x, y, .. } => *x != 0.0 || *y != 0.0,
            Event::TextInput { text, .. } => !text.is_empty(),
            _ => false,
        };
        if !activity {
            return false;
        }
        let changed = self.alpha > 0.0;
        self.last_input = now;
        self.alpha = 0.0;
        if changed {
            tracing::info!("Home brightness restored by input");
        }
        changed
    }

    pub fn tick(&mut self, now: Instant, visible: bool) -> bool {
        let previous = self.alpha;
        if visible != self.visible {
            self.visible = visible;
            self.last_input = now;
            self.alpha = 0.0;
        } else if visible {
            let elapsed = now
                .saturating_duration_since(self.last_input)
                .saturating_sub(self.delay);
            self.alpha = (elapsed.as_secs_f32() / FADE.as_secs_f32()).min(1.0) * DIM_ALPHA;
        }
        if previous == 0.0 && self.alpha > 0.0 {
            tracing::info!(
                idle_seconds = self.delay.as_secs(),
                "Home dimming after inactivity"
            );
        }
        previous != self.alpha
    }

    pub fn alpha(&self) -> f32 {
        self.alpha
    }

    pub fn animating(&self) -> bool {
        self.alpha > 0.0 && self.alpha < DIM_ALPHA
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sdl3::{event::WindowEvent, keyboard::Mod, mouse::MouseState};

    fn key(raw: u16) -> Event {
        Event::KeyDown {
            timestamp: 0,
            window_id: 1,
            keycode: None,
            scancode: None,
            keymod: Mod::NOMOD,
            repeat: false,
            which: 0,
            raw,
        }
    }

    #[test]
    fn dims_after_five_minutes_and_stops_animating() {
        let start = Instant::now();
        let mut dimmer = Dimmer::new(start, false);
        assert!(!dimmer.tick(start + Duration::from_secs(299), true));
        assert_eq!(dimmer.alpha(), 0.0);
        assert!(!dimmer.tick(start + IDLE_DELAY, true));
        assert!(dimmer.tick(start + IDLE_DELAY + FADE / 2, true));
        assert!((dimmer.alpha() - DIM_ALPHA / 2.0).abs() < 0.001);
        assert!(dimmer.animating());
        assert!(dimmer.tick(start + IDLE_DELAY + FADE, true));
        assert_eq!(dimmer.alpha(), DIM_ALPHA);
        assert!(!dimmer.animating());
        assert!(!dimmer.tick(start + Duration::from_secs(900), true));
    }

    #[test]
    fn pointer_motion_outside_controls_and_unhandled_keys_restore_immediately() {
        let start = Instant::now();
        let mut dimmer = Dimmer::new(start, false);
        let wake = start + IDLE_DELAY + FADE;
        dimmer.tick(wake, true);
        let motion = Event::MouseMotion {
            timestamp: 0,
            window_id: 1,
            which: 0,
            mousestate: MouseState::from_sdl_state(0),
            x: 0.0,
            y: 0.0,
            xrel: 1.0,
            yrel: 0.0,
        };
        assert!(dimmer.input(&motion, wake));
        assert_eq!(dimmer.alpha(), 0.0);
        assert!(!dimmer.tick(wake + IDLE_DELAY, true));
        dimmer.tick(wake + IDLE_DELAY + FADE, true);
        assert!(dimmer.input(&key(42), wake + IDLE_DELAY + FADE));
        assert_eq!(dimmer.alpha(), 0.0);
    }

    #[test]
    fn cursor_notifications_and_repaints_do_not_reset_inactivity() {
        let start = Instant::now();
        let mut dimmer = Dimmer::new(start, false);
        for event in [
            key(1198),
            key(1199),
            Event::Window {
                timestamp: 0,
                window_id: 1,
                win_event: WindowEvent::Exposed,
            },
        ] {
            assert!(!dimmer.input(&event, start + Duration::from_secs(299)));
        }
        dimmer.tick(start + IDLE_DELAY + FADE, true);
        assert_eq!(dimmer.alpha(), DIM_ALPHA);
    }

    #[test]
    fn returning_from_another_app_starts_bright_with_a_fresh_timer() {
        let start = Instant::now();
        let mut dimmer = Dimmer::new(start, false);
        dimmer.tick(start + IDLE_DELAY + FADE, true);
        assert!(dimmer.tick(start + Duration::from_secs(400), false));
        assert!(!dimmer.tick(start + Duration::from_secs(4000), false));
        dimmer.tick(start + Duration::from_secs(4001), true);
        assert_eq!(dimmer.alpha(), 0.0);
        assert!(!dimmer.tick(start + Duration::from_secs(4300), true));
        assert!(dimmer.tick(start + Duration::from_secs(4302), true));
    }
}
