mod dl;
pub mod luna;
pub mod wifi;

pub fn exit_held() -> bool {
    // SDL's webOS-only Exit scancode is outside rust-sdl3's enum.
    unsafe {
        let mut count = 0;
        let keys = sdl3::sys::keyboard::SDL_GetKeyboardState(&mut count);
        !keys.is_null() && count > 375 && *keys.add(375)
    }
}

pub fn clock(format: &std::ffi::CStr) -> String {
    unsafe {
        let now = libc::time(std::ptr::null_mut());
        let mut tm = std::mem::zeroed::<libc::tm>();
        if libc::localtime_r(&now, &mut tm).is_null() {
            return String::new();
        }
        let mut out = [0u8; 128];
        let n = libc::strftime(out.as_mut_ptr().cast(), out.len(), format.as_ptr(), &tm);
        String::from_utf8_lossy(&out[..n]).trim().to_string()
    }
}

/// Portable 12-hour label without strftime's leading zero.
pub fn time_label() -> String {
    clock(c"%I:%M %p").trim_start_matches('0').to_owned()
}
