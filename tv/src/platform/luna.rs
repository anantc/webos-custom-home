//! Minimal in-process LS2 adapter, based on Punktfunk's platform/webos/ls2.rs.
//! Each one-shot call owns its bus and callback context on one worker thread.
use super::dl;
use anyhow::{bail, Result};
use std::ffi::{c_char, c_int, c_ulong, c_void, CStr, CString};
use std::sync::{mpsc, OnceLock};
use std::time::{Duration, Instant};

#[repr(C)]
struct LsError {
    code: c_int,
    message: *mut c_char,
    rest: [usize; 14],
}
type Handle = *mut c_void;
type Filter = unsafe extern "C" fn(Handle, Handle, *mut c_void) -> bool;
struct Fns {
    init: unsafe extern "C" fn(*mut LsError),
    free: unsafe extern "C" fn(*mut LsError),
    register: unsafe extern "C" fn(*const c_char, *mut Handle, *mut LsError) -> bool,
    unregister: unsafe extern "C" fn(Handle, *mut LsError) -> bool,
    attach: unsafe extern "C" fn(Handle, Handle, *mut LsError) -> bool,
    call: unsafe extern "C" fn(
        Handle,
        *const c_char,
        *const c_char,
        Filter,
        Handle,
        *mut c_ulong,
        *mut LsError,
    ) -> bool,
    payload: unsafe extern "C" fn(Handle) -> *const c_char,
    context_new: unsafe extern "C" fn() -> Handle,
    iterate: unsafe extern "C" fn(Handle, c_int) -> c_int,
    unref: unsafe extern "C" fn(Handle),
}
fn fns() -> Result<&'static Fns> {
    static FNS: OnceLock<std::result::Result<Fns, String>> = OnceLock::new();
    dl::cached(&FNS, c"libluna-service2.so.3", |lib| {
        let glib = dl::Lib::open(c"libglib-2.0.so.0")?;
        Ok(Fns {
            init: lib.sym(c"LSErrorInit")?,
            free: lib.sym(c"LSErrorFree")?,
            register: lib.sym(c"LSRegister")?,
            unregister: lib.sym(c"LSUnregister")?,
            attach: lib.sym(c"LSGmainContextAttach")?,
            call: lib.sym(c"LSCallOneReply")?,
            payload: lib.sym(c"LSMessageGetPayload")?,
            context_new: glib.sym(c"g_main_context_new")?,
            iterate: glib.sym(c"g_main_context_iteration")?,
            unref: glib.sym(c"g_main_context_unref")?,
        })
    })
}

struct Error {
    raw: LsError,
    f: &'static Fns,
}
impl Error {
    fn new(f: &'static Fns) -> Self {
        let mut raw = LsError {
            code: 0,
            message: std::ptr::null_mut(),
            rest: [0; 14],
        };
        unsafe { (f.init)(&mut raw) };
        Self { raw, f }
    }
    fn message(&self) -> String {
        if self.raw.message.is_null() {
            return format!("Luna error {}", self.raw.code);
        }
        unsafe {
            CStr::from_ptr(self.raw.message)
                .to_string_lossy()
                .into_owned()
        }
    }
}
impl Drop for Error {
    fn drop(&mut self) {
        unsafe { (self.f.free)(&mut self.raw) }
    }
}

struct Reply {
    payload: Option<String>,
    f: &'static Fns,
}
unsafe extern "C" fn reply(_: Handle, message: Handle, context: Handle) -> bool {
    // Context stays boxed and live until LSUnregister stops every callback.
    let state = unsafe { &mut *context.cast::<Reply>() };
    let text = unsafe { (state.f.payload)(message) };
    state.payload = Some(if text.is_null() {
        "{}".into()
    } else {
        unsafe { CStr::from_ptr(text) }
            .to_string_lossy()
            .into_owned()
    });
    true
}

struct Bus {
    handle: Handle,
    context: Handle,
    f: &'static Fns,
    reply: Box<Reply>,
}
impl Bus {
    fn open() -> Result<Self> {
        let f = fns()?;
        let mut handle = std::ptr::null_mut();
        let mut err = Error::new(f);
        let registered = unsafe {
            (f.register)(
                c"com.anant.tvhome.native".as_ptr(),
                &mut handle,
                &mut err.raw,
            )
        };
        if !registered || handle.is_null() {
            // SAM can already own the application's name. Permissions still follow executable path.
            drop(err);
            err = Error::new(f);
            if !unsafe { (f.register)(std::ptr::null(), &mut handle, &mut err.raw) }
                || handle.is_null()
            {
                bail!("Cannot register with Luna: {}", err.message());
            }
        }
        let context = unsafe { (f.context_new)() };
        let bus = Self {
            handle,
            context,
            f,
            reply: Box::new(Reply { payload: None, f }),
        };
        if context.is_null() || !unsafe { (f.attach)(handle, context, &mut err.raw) } {
            bail!("Cannot attach Luna context: {}", err.message());
        }
        Ok(bus)
    }
    fn request(&mut self, uri: &CStr, payload: &str) -> Result<String> {
        let payload = CString::new(payload)?;
        let mut err = Error::new(self.f);
        let mut token = 0;
        let ok = unsafe {
            (self.f.call)(
                self.handle,
                uri.as_ptr(),
                payload.as_ptr(),
                reply,
                (&mut *self.reply as *mut Reply).cast(),
                &mut token,
                &mut err.raw,
            )
        };
        if !ok {
            bail!("Service call rejected: {}", err.message());
        }
        let started = Instant::now();
        while started.elapsed() < Duration::from_secs(5) {
            // Never pump or wait on the rendering thread.
            unsafe {
                (self.f.iterate)(self.context, 0);
            }
            if let Some(payload) = self.reply.payload.take() {
                return Ok(payload);
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        bail!("The TV did not reply within five seconds.")
    }
    fn launch(&mut self, id: &str) -> Result<()> {
        let payload = serde_json::json!({"id":id,"params":{}}).to_string();
        let response = self.request(c"luna://com.webos.applicationManager/launch", &payload)?;
        check_reply(&response)
    }
}
impl Drop for Bus {
    fn drop(&mut self) {
        // Unregister cancels outstanding calls before the boxed callback state is dropped.
        let mut err = Error::new(self.f);
        unsafe {
            (self.f.unregister)(self.handle, &mut err.raw);
            if !self.context.is_null() {
                (self.f.unref)(self.context);
            }
        }
    }
}
// A fresh bus per request ensures timeout cleanup cancels its pending callback.
pub(super) fn query(uri: &CStr, payload: &str) -> Result<String> {
    Bus::open()?.request(uri, payload)
}

fn check_reply(payload: &str) -> Result<()> {
    let value: serde_json::Value = serde_json::from_str(payload)?;
    if value["returnValue"] == true {
        return Ok(());
    }
    bail!(
        "{}",
        value["errorText"]
            .as_str()
            .unwrap_or("The TV refused to launch this app.")
    );
}

pub struct Launcher {
    pub requests: mpsc::SyncSender<(String, String)>,
    pub results: mpsc::Receiver<Result<(), String>>,
}
pub fn worker(preview: bool) -> Launcher {
    let (requests, queue) = mpsc::sync_channel::<(String, String)>(1);
    let (results_tx, results) = mpsc::channel();
    std::thread::spawn(move || {
        for (id, title) in queue {
            let start = Instant::now();
            let result = if preview {
                Ok(())
            } else {
                Bus::open()
                    .and_then(|mut bus| bus.launch(&id))
                    .map_err(|e| format!("{title}: {e:#}"))
            };
            tracing::info!(app = %id, title = %title, preview, elapsed_ms = start.elapsed().as_millis(), ok = result.is_ok(), "launch response");
            if results_tx.send(result).is_err() {
                break;
            }
        }
    });
    Launcher { requests, results }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn asynchronous_rejection_is_not_success() {
        assert!(check_reply(r#"{"returnValue": true}"#).is_ok());
        assert!(
            check_reply(r#"{"returnValue":false,"errorText":"Access denied"}"#)
                .unwrap_err()
                .to_string()
                .contains("Access denied")
        );
        assert!(check_reply("{}").is_err());
        assert!(check_reply("broken").is_err());
    }
}
