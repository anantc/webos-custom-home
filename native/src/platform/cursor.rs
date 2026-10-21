//! Keep SDL's cursor visibility aligned with LG's Magic Remote pointer.
use sdl3::event::Event;

pub fn visibility(event: &Event) -> Option<bool> {
    // SDL-webOS maps LG's XKB cursor show/hide codes 1206/1207 to these raw keys.
    // Their repeats and missing key-up describe visibility, not a held remote button.
    match event {
        Event::KeyDown { raw: 1198, .. } => Some(true),
        Event::KeyDown { raw: 1199, .. } => Some(false),
        _ => None,
    }
}
