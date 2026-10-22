mod calendar;
mod storage;
#[cfg(test)]
mod tests;

use calendar::{collection, Date, COLLECTIONS};
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::mpsc,
    time::{Duration, Instant, SystemTime},
};
use storage::{Bag, Loaded, Manifest, Photo, Store};

const ROTATION: Duration = Duration::from_secs(600);
const FADE: Duration = Duration::from_millis(1200);
const RETRY: Duration = Duration::from_secs(60);

enum Command {
    Load(String, String),
    Displayed(Photo),
}
enum Event {
    Cached(Loaded),
    Photo(anyhow::Result<Loaded>),
    Manifest(Manifest),
}

/// Tracks visible time only. Home's photo changes do not represent user activity.
#[derive(Default)]
struct VisibleClock {
    last: Option<Instant>,
    was_visible: bool,
    elapsed: Duration,
}
impl VisibleClock {
    fn tick(&mut self, now: Instant, visible: bool) -> Duration {
        let delta = if visible && self.was_visible {
            self.last
                .map(|t| now.saturating_duration_since(t))
                .unwrap_or_default()
        } else {
            Duration::ZERO
        };
        self.last = Some(now);
        self.was_visible = visible;
        self.elapsed += delta;
        delta
    }
}

pub struct View<'a> {
    pub current: Option<&'a Loaded>,
    pub previous: Option<&'a Loaded>,
    pub blend: f32,
}
pub struct Monitor {
    requests: mpsc::SyncSender<Command>,
    results: mpsc::Receiver<Event>,
    manifest: Manifest,
    current: Option<Loaded>,
    previous: Option<Loaded>,
    next: Option<Loaded>,
    timer: VisibleClock,
    fade: Duration,
    interval: Duration,
    pending: bool,
    retry_at: Instant,
    desired: String,
    calendar_at: Instant,
    preview_collection: Option<String>,
    preview_date: Option<Date>,
}
impl Monitor {
    pub fn new(root: PathBuf, preview: bool) -> Self {
        let preview_base = preview
            .then(|| std::env::var("TVHOME_PREVIEW_BACKGROUND_BASE").ok())
            .flatten();
        let preview_collection = preview
            .then(|| std::env::var("TVHOME_PREVIEW_BACKGROUND_COLLECTION").ok())
            .flatten()
            .filter(|c| COLLECTIONS.contains(&c.as_str()));
        let preview_date = preview
            .then(|| std::env::var("TVHOME_PREVIEW_BACKGROUND_DATE").ok())
            .flatten()
            .and_then(|s| Date::parse(&s));
        let interval = if preview {
            std::env::var("TVHOME_PREVIEW_BACKGROUND_SECONDS")
                .ok()
                .and_then(|s| s.parse::<u64>().ok())
                .filter(|s| *s >= 3)
                .map(Duration::from_secs)
                .unwrap_or(ROTATION)
        } else {
            ROTATION
        };
        let (tx, requests) = mpsc::sync_channel(4);
        let (results, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let run = || -> anyhow::Result<()> {
                let fallback = (!preview)
                    .then(|| std::env::temp_dir().join("com.anant.tvhome.native-backgrounds"));
                let mut store = Store::open(root, preview_base, fallback)?;
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()?;
                let _ = results.send(Event::Manifest(store.manifest.clone()));
                if let Some(cached) = store.restore() {
                    let _ = results.send(Event::Cached(cached));
                }
                let seed = SystemTime::now()
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_nanos() as u64;
                let mut bag = Bag::new(seed);
                let mut failed = HashMap::<String, Instant>::new();
                while let Ok(command) = requests.recv() {
                    match command {
                        Command::Displayed(photo) => {
                            if let Err(error) = store.displayed(&photo) {
                                tracing::warn!(%error,"Could not persist background selection");
                            }
                        }
                        Command::Load(collection, exclude) => {
                            match runtime.block_on(store.refresh()) {
                                Ok(true) => {
                                    failed.clear();
                                    bag =
                                        Bag::new(seed.wrapping_add(store.manifest.revision as u64));
                                    let _ = results.send(Event::Manifest(store.manifest.clone()));
                                }
                                Ok(false) => (),
                                Err(error) => {
                                    tracing::warn!(%error,"Background manifest unavailable; keeping reviewed list")
                                }
                            }
                            let mut loaded = Err(anyhow::anyhow!(
                                "No available photo in {collection}; retaining current background"
                            ));
                            let attempts = store
                                .manifest
                                .photos
                                .iter()
                                .filter(|p| p.collection == collection)
                                .count();
                            for _ in 0..attempts {
                                let Ok(photo) =
                                    bag.next(&store.manifest.photos, &collection, &exclude)
                                else {
                                    break;
                                };
                                if failed
                                    .get(&photo.file)
                                    .is_some_and(|until| Instant::now() < *until)
                                {
                                    continue;
                                }
                                match runtime.block_on(store.load(&photo)) {
                                    Ok(image) => {
                                        loaded = Ok(image);
                                        break;
                                    }
                                    Err(error) => {
                                        tracing::warn!(id=%photo.id,error=format_args!("{error:#}"),"Background skipped");
                                        failed.insert(
                                            photo.file,
                                            Instant::now() + Duration::from_secs(900),
                                        );
                                    }
                                }
                            }
                            if results.send(Event::Photo(loaded)).is_err() {
                                break;
                            }
                        }
                    }
                }
                Ok(())
            };
            if let Err(error) = run() {
                let _ = results.send(Event::Photo(Err(error)));
            }
        });
        let now = Instant::now();
        Self {
            requests: tx,
            results: rx,
            manifest: Manifest::seed(),
            current: None,
            previous: None,
            next: None,
            timer: VisibleClock::default(),
            fade: FADE,
            interval,
            pending: false,
            retry_at: now,
            desired: String::new(),
            calendar_at: now,
            preview_collection,
            preview_date,
        }
    }
    pub fn view(&self) -> View<'_> {
        View {
            current: self.current.as_ref(),
            previous: self.previous.as_ref(),
            blend: (self.fade.as_secs_f32() / FADE.as_secs_f32()).min(1.0),
        }
    }
    pub fn animating(&self) -> bool {
        self.previous.is_some() && self.fade < FADE
    }
    fn show(&mut self, loaded: Loaded) {
        tracing::info!(id=%loaded.photo.id,collection=%loaded.photo.collection,"Background displayed");
        let _ = self
            .requests
            .try_send(Command::Displayed(loaded.photo.clone()));
        self.previous = self.current.replace(loaded);
        self.fade = if self.previous.is_some() {
            Duration::ZERO
        } else {
            FADE
        };
        self.timer.elapsed = Duration::ZERO;
    }
    pub fn tick(&mut self, now: Instant, visible: bool) -> bool {
        let delta = self.timer.tick(now, visible);
        let mut dirty = false;
        if self.previous.is_some() {
            self.fade += delta;
            if self.fade >= FADE {
                self.previous = None;
                dirty = true;
            }
        }
        // Receive completed work in the background, but present it only when Home is visible.
        for event in self.results.try_iter().collect::<Vec<_>>() {
            match event {
                Event::Manifest(manifest) => {
                    self.manifest = manifest;
                    self.calendar_at = now;
                }
                Event::Cached(loaded) => {
                    if self.current.is_none() {
                        self.current = Some(loaded);
                        dirty = true;
                    }
                }
                Event::Photo(result) => {
                    self.pending = false;
                    match result {
                        Ok(loaded) => self.next = Some(loaded),
                        Err(error) => {
                            tracing::warn!(%error,"Background preload unavailable");
                            self.retry_at = now + RETRY;
                        }
                    }
                }
            }
        }
        if !visible {
            return dirty;
        }
        if now >= self.calendar_at {
            let date = self
                .preview_date
                .or_else(|| Date::parse(&crate::platform::clock(c"%Y-%m-%d")));
            let desired = self.preview_collection.clone().unwrap_or_else(|| {
                date.map(|d| collection(d, &self.manifest.holidays).into())
                    .unwrap_or_else(|| "autumn".into())
            });
            if self.desired != desired {
                tracing::info!(collection=%desired,"Background collection selected");
                self.desired = desired;
                self.retry_at = now;
            }
            self.calendar_at = now + Duration::from_secs(60);
        }
        if self.next.as_ref().is_some_and(|n| {
            n.photo.collection != self.desired
                || self
                    .current
                    .as_ref()
                    .is_some_and(|c| c.photo.id == n.photo.id)
        }) {
            self.next = None;
        }
        let change_collection = self
            .current
            .as_ref()
            .is_some_and(|c| c.photo.collection != self.desired);
        if self.next.is_some()
            && (self.current.is_none() || change_collection || self.timer.elapsed >= self.interval)
            && !self.animating()
        {
            let next = self.next.take().expect("ready photo");
            self.show(next);
            dirty = true;
        }
        if !self.pending && self.next.is_none() && now >= self.retry_at {
            let exclude = self
                .current
                .as_ref()
                .map(|p| p.photo.id.clone())
                .unwrap_or_default();
            if self
                .requests
                .try_send(Command::Load(self.desired.clone(), exclude))
                .is_ok()
            {
                self.pending = true;
            } else {
                self.retry_at = now + RETRY;
            }
        }
        dirty
    }
}
