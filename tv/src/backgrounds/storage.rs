use super::calendar::{Date, Holiday, COLLECTIONS};
use anyhow::{bail, ensure, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    io::Cursor,
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};

pub const MANIFEST_URL: &str =
    "https://github.com/anantc/webos-custom-home/releases/download/backgrounds-v1/manifest.json";
const ASSET_PREFIX: &str =
    "https://github.com/anantc/webos-custom-home/releases/download/backgrounds-v1/";
const MAX_IMAGE: usize = 4 * 1024 * 1024;
const MAX_MANIFEST: usize = 256 * 1024;
const MANIFEST_REFRESH: Duration = Duration::from_secs(900);

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Photo {
    pub id: String,
    pub tags: Vec<String>,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub caption: Option<String>,
    pub author: String,
    pub source: String,
    pub license: String,
    pub file: String,
    pub sha256: String,
    pub bytes: usize,
    pub url: String,
}
impl Photo {
    pub fn matches_tag(&self, tag: &str) -> bool {
        self.tags.iter().any(|t| t == tag)
    }
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Manifest {
    pub version: u32,
    pub revision: u32,
    pub holidays: Vec<Holiday>,
    pub photos: Vec<Photo>,
}
impl Manifest {
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        ensure!(bytes.len() <= MAX_MANIFEST, "Manifest too large");
        let m: Self = serde_json::from_slice(bytes)?;
        ensure!(
            m.version == 2 && m.revision > 0,
            "Unsupported background manifest"
        );
        ensure!(
            !m.photos.is_empty() && m.photos.len() <= 120 && m.holidays.len() <= 100,
            "Invalid collection size"
        );
        let mut ids = HashSet::new();
        for p in &m.photos {
            ensure!(ids.insert(&p.id), "Duplicate background ID");
            ensure!(
                !p.id.is_empty()
                    && p.id.len() <= 12
                    && p.id.bytes().all(|c| c.is_ascii_alphanumeric()),
                "Invalid photo ID"
            );
            let mut tags = HashSet::new();
            ensure!(
                !p.tags.is_empty()
                    && p.tags.len() <= 16
                    && p.tags.iter().all(|t| {
                        !t.is_empty()
                            && t.len() <= 32
                            && t.bytes().all(|c| c.is_ascii_lowercase() || c == b'-')
                            && tags.insert(t)
                    })
                    && COLLECTIONS.iter().any(|t| p.matches_tag(t)),
                "Invalid photo tags or missing season/holiday tag"
            );
            ensure!(
                p.sha256.len() == 64
                    && p.sha256
                        .bytes()
                        .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()),
                "Invalid checksum"
            );
            ensure!(
                p.file == format!("{}-{}.jpg", p.id, &p.sha256[..16]),
                "Invalid filename"
            );
            ensure!(
                p.url == format!("{ASSET_PREFIX}{}", p.file),
                "Untrusted photo origin"
            );
            ensure!(p.bytes > 0 && p.bytes <= MAX_IMAGE, "Invalid photo size");
            ensure!(
                !p.author.is_empty()
                    && p.author.chars().count() <= 100
                    && p.title.chars().count() <= 160
                    && p.caption.as_ref().is_none_or(|c| {
                        !c.trim().is_empty()
                            && c.chars().count() <= 160
                            && !c.chars().any(char::is_control)
                    }),
                "Invalid credit"
            );
            ensure!(
                p.source.starts_with("https://unsplash.com/photos/")
                    && p.license == "https://unsplash.com/license",
                "Invalid source attribution"
            );
        }
        for c in COLLECTIONS {
            ensure!(
                (1..=12).contains(&m.photos.iter().filter(|p| p.matches_tag(c)).count()),
                "Missing or oversized collection: {c}"
            );
        }
        for h in &m.holidays {
            let start = Date::parse(&h.start).context("Invalid holiday start")?;
            let end = Date::parse(&h.end).context("Invalid holiday end")?;
            ensure!(
                ["diwali", "holi"].contains(&h.collection.as_str())
                    && start <= end
                    && start.0 == end.0
                    && start.1 == end.1
                    && end.2 - start.2 <= 13,
                "Invalid lunar holiday window"
            );
        }
        Ok(m)
    }
    pub fn seed() -> Self {
        Self::parse(include_bytes!("../../backgrounds/manifest.json"))
            .expect("validated bundled photo manifest")
    }
}

pub struct Loaded {
    pub photo: Photo,
    pub image: skia_safe::Image,
}

pub struct Store {
    pub root: PathBuf,
    pub manifest: Manifest,
    client: reqwest::Client,
    preview_base: Option<String>,
    last_good: Option<String>,
    refresh_after: SystemTime,
}
fn trusted_redirect(url: &reqwest::Url) -> bool {
    url.scheme() == "https"
        && matches!(
            url.host_str(),
            Some(
                "github.com"
                    | "release-assets.githubusercontent.com"
                    | "objects.githubusercontent.com"
                    | "raw.githubusercontent.com"
            )
        )
}
impl Store {
    pub fn open(
        root: PathBuf,
        preview_base: Option<String>,
        fallback: Option<PathBuf>,
    ) -> Result<Self> {
        match Self::new(root, preview_base.clone()) {
            Ok(store) => Ok(store),
            Err(error) => {
                let denied = error.chain().any(|cause| {
                    cause.downcast_ref::<std::io::Error>().is_some_and(|e| {
                        matches!(
                            e.kind(),
                            std::io::ErrorKind::PermissionDenied
                                | std::io::ErrorKind::ReadOnlyFilesystem
                        )
                    })
                });
                if denied {
                    if let Some(path) = fallback {
                        tracing::warn!(%error, path = %path.display(), "App storage is unwritable; photos use a temporary cache cleared on reboot");
                        return Self::new(path, preview_base);
                    }
                }
                Err(error)
            }
        }
    }
    pub fn new(root: PathBuf, preview_base: Option<String>) -> Result<Self> {
        if let Some(base) = &preview_base {
            let url = reqwest::Url::parse(base)?;
            ensure!(
                url.scheme() == "http"
                    && matches!(url.host_str(), Some("127.0.0.1" | "localhost"))
                    && url.username().is_empty()
                    && url.password().is_none(),
                "Preview origin must be loopback HTTP"
            );
        }
        std::fs::create_dir_all(&root)?;
        // An existing directory can become unwritable after an LG app update.
        let nonce = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)?
            .as_nanos();
        let probe = root.join(format!(".write-check-{}-{nonce}", std::process::id()));
        let file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&probe)?;
        drop(file);
        std::fs::remove_file(probe)?;
        let seed = Manifest::seed();
        let cached = std::fs::read(root.join("manifest.json"))
            .ok()
            .and_then(|b| Manifest::parse(&b).ok())
            .filter(|m| m.revision >= seed.revision);
        let client = reqwest::Client::builder()
            // The worker parks its runtime between photos; do not retain idle sockets.
            .pool_max_idle_per_host(0)
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(30))
            .redirect(reqwest::redirect::Policy::custom(|a| {
                if a.previous().len() >= 4 || !trusted_redirect(a.url()) {
                    a.error("Untrusted background redirect")
                } else {
                    a.follow()
                }
            }))
            .user_agent("tvhome-backgrounds/1")
            .build()?;
        Ok(Self {
            root,
            manifest: cached.unwrap_or(seed),
            client,
            preview_base,
            last_good: None,
            refresh_after: SystemTime::UNIX_EPOCH,
        })
    }
    async fn download(&self, url: &str, limit: usize) -> Result<Vec<u8>> {
        let mut response = self.client.get(url).send().await?.error_for_status()?;
        ensure!(
            response.content_length().is_none_or(|n| n <= limit as u64),
            "Background response too large"
        );
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await? {
            ensure!(
                bytes.len() + chunk.len() <= limit,
                "Background response too large"
            );
            bytes.extend_from_slice(&chunk);
        }
        Ok(bytes)
    }
    pub async fn refresh(&mut self, now: SystemTime) -> Result<bool> {
        if now < self.refresh_after {
            return Ok(false);
        }
        // A failed origin is retried later; it never invalidates the last good manifest.
        self.refresh_after = now + MANIFEST_REFRESH;
        let url = self
            .preview_base
            .as_ref()
            .map(|b| format!("{b}manifest.json"))
            .unwrap_or_else(|| MANIFEST_URL.into());
        let bytes = self.download(&url, MAX_MANIFEST).await?;
        let manifest = Manifest::parse(&bytes)?;
        ensure!(
            manifest.revision >= self.manifest.revision,
            "Older background manifest ignored"
        );
        atomic(&self.root.join("manifest.json"), &bytes)?;
        let changed = manifest.revision != self.manifest.revision;
        self.manifest = manifest;
        tracing::info!(
            revision = self.manifest.revision,
            changed,
            "Background manifest checked"
        );
        Ok(changed)
    }
    fn decode(photo: &Photo, bytes: &[u8]) -> Result<skia_safe::Image> {
        ensure!(
            bytes.len() == photo.bytes && format!("{:x}", Sha256::digest(bytes)) == photo.sha256,
            "Photo checksum mismatch"
        );
        let mut reader =
            image::ImageReader::with_format(Cursor::new(bytes), image::ImageFormat::Jpeg);
        let mut limits = image::Limits::default();
        limits.max_image_width = Some(1920);
        limits.max_image_height = Some(1080);
        limits.max_alloc = Some(24 * 1024 * 1024);
        reader.limits(limits);
        let image = reader.decode()?;
        ensure!(
            image.width() == 1920 && image.height() == 1080,
            "Photo must be 1920×1080"
        );
        let rgba = image.into_rgba8();
        crate::render::rgba_image(rgba.as_raw(), 1920, 1080)
    }
    pub fn restore(&mut self) -> Option<Loaded> {
        #[derive(Deserialize)]
        struct SavedPhoto {
            file: String,
        }
        let photo: SavedPhoto =
            serde_json::from_slice(&std::fs::read(self.root.join("last-good.json")).ok()?).ok()?;
        // Reuse only photos still present in the reviewed manifest, not arbitrary disk paths.
        let photo = self
            .manifest
            .photos
            .iter()
            .find(|p| p.file == photo.file)?
            .clone();
        let bytes = std::fs::read(self.root.join(&photo.file)).ok()?;
        let image = Self::decode(&photo, &bytes).ok()?;
        self.last_good = Some(photo.file.clone());
        Some(Loaded { photo, image })
    }
    pub fn displayed(&mut self, photo: &Photo) -> Result<()> {
        atomic(
            &self.root.join("last-good.json"),
            &serde_json::to_vec(photo)?,
        )?;
        self.last_good = Some(photo.file.clone());
        Ok(())
    }
    pub async fn load(&mut self, photo: &Photo) -> Result<Loaded> {
        let path = self.root.join(&photo.file);
        if let Ok(bytes) = std::fs::read(&path) {
            if let Ok(image) = Self::decode(photo, &bytes) {
                self.retain_pair(&photo.file);
                return Ok(Loaded {
                    photo: photo.clone(),
                    image,
                });
            }
        }
        let url = self
            .preview_base
            .as_ref()
            .map(|b| format!("{b}{}", photo.file))
            .unwrap_or_else(|| photo.url.clone());
        let bytes = self.download(&url, MAX_IMAGE).await?;
        let image = Self::decode(photo, &bytes)?;
        atomic(&path, &bytes)?;
        self.retain_pair(&photo.file);
        Ok(Loaded {
            photo: photo.clone(),
            image,
        })
    }
    fn retain_pair(&self, ready: &str) {
        // Only called after verification and successful storage. A failed fetch
        // leaves both existing photos intact, including the previous fallback.
        let prune = || -> Result<()> {
            for entry in std::fs::read_dir(&self.root)? {
                let entry = entry?;
                let name = entry.file_name().to_string_lossy().into_owned();
                if name.ends_with(".jpg") && name != ready && self.last_good.as_ref() != Some(&name)
                {
                    std::fs::remove_file(entry.path())?;
                }
            }
            Ok(())
        };
        if let Err(error) = prune() {
            tracing::warn!(%error,"Could not discard old background photos");
        }
    }
}
fn atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    use std::io::Write;
    let temp = path.with_extension("tmp");
    let mut file = std::fs::File::create(&temp)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    std::fs::rename(temp, path)?;
    Ok(())
}

pub struct Bag {
    remaining: Vec<String>,
    collection: String,
    seed: u64,
}
impl Bag {
    pub fn new(seed: u64) -> Self {
        Self {
            remaining: Vec::new(),
            collection: String::new(),
            seed: seed.max(1),
        }
    }
    pub fn next(&mut self, photos: &[Photo], collection: &str, exclude: &str) -> Result<Photo> {
        if self.collection != collection {
            self.remaining.clear();
            self.collection = collection.into();
        }
        if self.remaining.is_empty() {
            self.remaining = photos
                .iter()
                .filter(|p| p.matches_tag(collection))
                .map(|p| p.id.clone())
                .collect();
            for i in (1..self.remaining.len()).rev() {
                self.seed ^= self.seed << 13;
                self.seed ^= self.seed >> 7;
                self.seed ^= self.seed << 17;
                self.remaining.swap(i, self.seed as usize % (i + 1));
            }
            if self.remaining.len() > 1 && self.remaining.last().is_some_and(|id| id == exclude) {
                let last = self.remaining.len() - 1;
                self.remaining.swap(0, last);
            }
        }
        while let Some(id) = self.remaining.pop() {
            if id != exclude {
                if let Some(p) = photos
                    .iter()
                    .find(|p| p.id == id && p.matches_tag(collection))
                {
                    return Ok(p.clone());
                }
            }
        }
        bail!("No alternative photo in {collection}")
    }
}
