//! LG owns pointer visibility. The bundled SDL patch synchronizes local cursor state
//! before keyboard repeat handling, without changing the compositor's global state.
use sdl3::event::Event;

pub fn configure() {
    sdl3::hint::set("TVHOME_WEBOS_COMPOSITOR_CURSOR", "1");
}

pub fn is_notification(event: &Event) -> bool {
    // Defensive filtering also covers an older library or queued key-up events.
    matches!(
        event,
        Event::KeyDown {
            raw: 1198 | 1199,
            ..
        } | Event::KeyUp {
            raw: 1198 | 1199,
            ..
        }
    )
}
