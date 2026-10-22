use super::*;
use sha2::{Digest, Sha256};
use std::{
    io::{Read, Write},
    net::TcpListener,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc,
    },
};

static TEMP_ID: AtomicUsize = AtomicUsize::new(0);
struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        let p = std::env::temp_dir().join(format!(
            "tvhome-background-{}-{}",
            std::process::id(),
            TEMP_ID.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&p).unwrap();
        Self(p)
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
#[cfg(target_os = "linux")]
fn read_only_storage_falls_back_and_restores_the_cached_manifest() {
    let temporary = Temp::new();
    // sysfs is read-only even for root in the build/preview container.
    let mut store = Store::open(
        PathBuf::from("/sys/kernel"),
        None,
        Some(temporary.0.join("fallback")),
    )
    .unwrap();
    assert_eq!(store.root, temporary.0.join("fallback"));
    assert!(Store::open(PathBuf::from("/sys/kernel"), None, None).is_err());
    let manifest = store.manifest.clone();
    store.manifest.photos[0].title = "Cached manifest".into();
    std::fs::write(
        store.root.join("manifest.json"),
        serde_json::to_vec(&store.manifest).unwrap(),
    )
    .unwrap();
    let reopened = Store::open(PathBuf::from("/sys/kernel"), None, Some(store.root)).unwrap();
    assert_eq!(reopened.manifest.photos[0].title, "Cached manifest");
    assert_eq!(reopened.manifest.photos.len(), manifest.photos.len());
}

#[test]
fn writable_app_cache_is_preferred_and_invalid_paths_do_not_silently_fall_back() {
    let temp = Temp::new();
    let app = temp.0.join("app");
    let fallback = temp.0.join("fallback");
    assert_eq!(
        Store::open(app.clone(), None, Some(fallback.clone()))
            .unwrap()
            .root,
        app
    );
    assert!(!fallback.exists());
    let invalid = temp.0.join("file");
    std::fs::write(&invalid, b"not a directory").unwrap();
    assert!(Store::open(invalid, None, Some(fallback.clone())).is_err());
    assert!(!fallback.exists());
}
struct Server {
    base: String,
    stop: Arc<AtomicBool>,
    requests: Arc<AtomicUsize>,
}
impl Server {
    fn new(status: u16, bytes: Vec<u8>) -> Self {
        Self::with_idle_close(status, bytes, false)
    }
    fn with_idle_close(status: u16, bytes: Vec<u8>, idle_close: bool) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let base = format!("http://{}/", listener.local_addr().unwrap());
        let stop = Arc::new(AtomicBool::new(false));
        let end = stop.clone();
        let requests = Arc::new(AtomicUsize::new(0));
        let count = requests.clone();
        std::thread::spawn(move || {
            while !end.load(Ordering::Relaxed) {
                if let Ok((mut stream, _)) = listener.accept() {
                    stream
                        .set_read_timeout(Some(Duration::from_secs(1)))
                        .unwrap();
                    let _ = stream.read(&mut [0u8; 4096]);
                    count.fetch_add(1, Ordering::Relaxed);
                    let _=write!(stream,"HTTP/1.1 {status} Response\r\nContent-Length: {}\r\nConnection: {}\r\n\r\n",bytes.len(), if idle_close {"keep-alive"} else {"close"});
                    let _ = stream.write_all(&bytes);
                    if idle_close {
                        std::thread::sleep(Duration::from_millis(50));
                    }
                } else {
                    std::thread::sleep(Duration::from_millis(5));
                }
            }
        });
        Self {
            base,
            stop,
            requests,
        }
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}
fn fixture() -> (Photo, Vec<u8>) {
    let pixels = image::RgbImage::from_pixel(1920, 1080, image::Rgb([44, 88, 120]));
    let mut bytes = Vec::new();
    image::codecs::jpeg::JpegEncoder::new(&mut bytes)
        .encode_image(&pixels)
        .unwrap();
    let mut p = Manifest::seed().photos[0].clone();
    p.sha256 = format!("{:x}", Sha256::digest(&bytes));
    p.bytes = bytes.len();
    p.file = format!("{}-{}.jpg", p.id, &p.sha256[..16]);
    p.url = format!(
        "https://github.com/anantc/webos-custom-home/releases/download/backgrounds-v1/{}",
        p.file
    );
    (p, bytes)
}
fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}

#[test]
fn approved_manifest_has_every_collection_and_replaced_sp2() {
    let m = Manifest::seed();
    assert_eq!(m.photos.len(), 55);
    for c in COLLECTIONS {
        assert_eq!(
            m.photos.iter().filter(|p| p.collection == c).count(),
            if ["spring", "summer", "monsoon", "autumn", "winter"].contains(&c) {
                6
            } else {
                5
            }
        );
    }
    assert!(m
        .photos
        .iter()
        .find(|p| p.id == "SP2")
        .unwrap()
        .source
        .ends_with("QF2kkrpmx34"));
    assert_eq!(collection(Date(2026, 11, 8), &m.holidays), "diwali");
    assert_eq!(collection(Date(2026, 3, 3), &m.holidays), "holi");
    assert_eq!(collection(Date(2027, 3, 22), &m.holidays), "holi");
    assert_eq!(collection(Date(2027, 10, 29), &m.holidays), "diwali");
    // Unknown years do not inherit a lunar festival's old dates.
    assert_eq!(collection(Date(2029, 11, 8), &m.holidays), "autumn");
}
#[test]
fn manifest_rejects_paths_foreign_origins_and_duplicate_ids() {
    for mutation in 0..3 {
        let mut m = Manifest::seed();
        match mutation {
            0 => m.photos[0].file = "../preferences.json".into(),
            1 => m.photos[0].url = "https://example.com/photo.jpg".into(),
            _ => m.photos[0].id = m.photos[1].id.clone(),
        };
        assert!(Manifest::parse(&serde_json::to_vec(&m).unwrap()).is_err());
    }
}
#[test]
fn shuffle_exhausts_collection_without_repeating_at_boundary() {
    let m = Manifest::seed();
    let mut bag = Bag::new(98);
    let mut last = String::new();
    for _ in 0..20 {
        let mut cycle = std::collections::HashSet::new();
        for _ in 0..6 {
            let p = bag.next(&m.photos, "autumn", &last).unwrap();
            assert_ne!(p.id, last);
            assert!(cycle.insert(p.id.clone()));
            last = p.id;
        }
    }
    assert_eq!(
        bag.next(&m.photos, "diwali", &last).unwrap().collection,
        "diwali"
    );
}
#[test]
fn rotation_counts_foreground_time_and_pauses_in_background() {
    let now = Instant::now();
    let mut timer = VisibleClock::default();
    timer.tick(now, true);
    timer.tick(now + Duration::from_secs(120), true);
    timer.tick(now + Duration::from_secs(120), false);
    timer.tick(now + Duration::from_secs(3600), false);
    timer.tick(now + Duration::from_secs(3600), true);
    assert_eq!(timer.elapsed, Duration::from_secs(120));
    timer.tick(now + Duration::from_secs(4080), true);
    assert_eq!(timer.elapsed, ROTATION);
}
#[test]
fn downloads_verified_photo_and_restores_offline_without_network() {
    let temp = Temp::new();
    let (photo, bytes) = fixture();
    let server = Server::new(200, bytes);
    let mut store = Store::new(temp.0.clone(), Some(server.base.clone())).unwrap();
    store.manifest.photos[0] = photo.clone();
    let loaded = runtime().block_on(store.load(&photo)).unwrap();
    store.displayed(&loaded.photo).unwrap();
    assert_eq!(server.requests.load(Ordering::Relaxed), 1);
    drop(server);
    assert_eq!(store.restore().unwrap().photo.id, photo.id);
    assert!(runtime().block_on(store.load(&photo)).is_ok());
}
#[test]
fn bad_download_keeps_last_good_and_does_not_cache_error_body() {
    for (status, body) in [(404, b"not found".to_vec()), (200, b"not a JPEG".to_vec())] {
        let temp = Temp::new();
        let (photo, bytes) = fixture();
        let server = Server::new(status, body);
        let mut store = Store::new(temp.0.clone(), Some(server.base.clone())).unwrap();
        store.manifest.photos[0] = photo.clone();
        std::fs::write(temp.0.join(&photo.file), bytes).unwrap();
        store.displayed(&photo).unwrap();
        let other = store.manifest.photos[1].clone();
        assert!(runtime().block_on(store.load(&other)).is_err());
        assert!(!temp.0.join(other.file).exists());
        assert!(store.restore().is_some());
    }
}
#[test]
fn corrupt_cache_is_replaced_only_after_valid_download() {
    let temp = Temp::new();
    let (photo, bytes) = fixture();
    let server = Server::new(200, bytes.clone());
    let mut store = Store::new(temp.0.clone(), Some(server.base.clone())).unwrap();
    std::fs::write(temp.0.join(&photo.file), b"corrupt").unwrap();
    assert!(runtime().block_on(store.load(&photo)).is_ok());
    assert_eq!(std::fs::read(temp.0.join(&photo.file)).unwrap(), bytes);
}
#[test]
fn manifest_failure_backs_off_and_preserves_reviewed_photos() {
    let temp = Temp::new();
    let server = Server::new(404, b"gone".to_vec());
    let mut store = Store::new(temp.0.clone(), Some(server.base.clone())).unwrap();
    let rt = runtime();
    assert!(rt.block_on(store.refresh()).is_err());
    assert!(!rt.block_on(store.refresh()).unwrap());
    assert_eq!(store.manifest.photos.len(), 55);
    assert_eq!(server.requests.load(Ordering::Relaxed), 1);
}

#[test]
#[ignore = "requires prepared photos: TVHOME_BACKGROUND_ASSETS=tv/dist/backgrounds-v1"]
fn all_prepared_photos_have_valid_hashes_and_decode_at_tv_resolution() {
    let root =
        PathBuf::from(std::env::var("TVHOME_BACKGROUND_ASSETS").expect("photo asset directory"));
    let mut store = Store::new(root, None).unwrap();
    let rt = runtime();
    for photo in store.manifest.photos.clone() {
        assert!(
            store.root.join(&photo.file).is_file(),
            "missing {}",
            photo.id
        );
        rt.block_on(store.load(&photo))
            .unwrap_or_else(|e| panic!("{}: {e}", photo.id));
    }
}

#[test]
fn next_download_succeeds_after_server_closes_an_idle_connection() {
    let temp = Temp::new();
    let (photo, bytes) = fixture();
    let server = Server::with_idle_close(200, bytes, true);
    let mut store = Store::new(temp.0.clone(), Some(server.base.clone())).unwrap();
    let rt = runtime();
    rt.block_on(store.load(&photo)).unwrap();
    // The worker parks its runtime between downloads, just as it does between rotations.
    std::thread::sleep(Duration::from_millis(200));
    let mut next = photo.clone();
    next.id = "SP2".into();
    next.file = format!("{}-{}.jpg", next.id, &next.sha256[..16]);
    rt.block_on(store.load(&next))
        .expect("a new request after an idle connection closes");
    assert_eq!(server.requests.load(Ordering::Relaxed), 2);
}

#[test]
fn ready_photos_wait_while_hidden_then_fade_without_a_black_frame() {
    let now = Instant::now();
    let (requests, _commands) = mpsc::sync_channel(20);
    let (events, results) = mpsc::channel();
    let manifest = Manifest::seed();
    let photos: Vec<_> = manifest
        .photos
        .iter()
        .filter(|p| p.collection == "autumn")
        .cloned()
        .collect();
    let loaded = |photo: Photo| Loaded {
        photo,
        image: crate::render::rgba_image(&[40, 80, 120, 255], 1, 1).unwrap(),
    };
    let mut m = Monitor {
        requests,
        results,
        manifest,
        current: None,
        previous: None,
        next: None,
        timer: VisibleClock::default(),
        fade: FADE,
        interval: ROTATION,
        pending: false,
        retry_at: now,
        desired: String::new(),
        calendar_at: now,
        preview_collection: None,
        preview_date: Some(Date(2026, 10, 22)),
    };
    events
        .send(Event::Photo(Ok(loaded(photos[0].clone()))))
        .unwrap();
    m.tick(now, true);
    assert_eq!(m.view().current.unwrap().photo.id, photos[0].id);
    // A startup request can race the disk-cache restore; never queue the same photo again.
    events
        .send(Event::Photo(Ok(loaded(photos[0].clone()))))
        .unwrap();
    m.tick(now + Duration::from_secs(1), true);
    assert!(m.next.is_none());
    m.tick(now + Duration::from_secs(60), true);
    m.tick(now + Duration::from_secs(60), false);
    events
        .send(Event::Photo(Ok(loaded(photos[1].clone()))))
        .unwrap();
    m.tick(now + Duration::from_secs(3600), false);
    assert_eq!(m.view().current.unwrap().photo.id, photos[0].id);
    m.tick(now + Duration::from_secs(3600), true);
    assert!(!m.animating());
    m.tick(now + Duration::from_secs(4140), true);
    assert!(m.animating());
    assert!(m.view().previous.is_some());
    assert_eq!(m.view().blend, 0.0);
    m.tick(now + Duration::from_millis(4_140_600), true);
    assert!((m.view().blend - 0.5).abs() < 0.01);
    m.tick(now + Duration::from_millis(4_141_200), true);
    assert!(!m.animating());
    assert!(m.view().previous.is_none());
    assert_eq!(m.view().current.unwrap().photo.id, photos[1].id);
}
