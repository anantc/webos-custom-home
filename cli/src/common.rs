use regex::Regex;
use reqwest::{redirect::Policy, Client, Url};
use std::{
    collections::BTreeMap,
    fmt, fs,
    io::Write,
    os::unix::fs::{DirBuilderExt, PermissionsExt},
    path::{Path, PathBuf},
    time::Duration,
};

// Network errors deliberately carry only fixed messages; upstream errors may contain secrets.
#[derive(Debug, Clone, PartialEq)]
pub enum Error {
    Fault(&'static str),
    Unavailable,
    Busy,
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Fault(s) => f.write_str(s),
            Self::Busy => f.write_str("This operation is already running. Stop the foreground command or this TV's service before retrying."),
            Self::Unavailable => f.write_str("TV unavailable; retrying when it returns."),
        }
    }
}
impl std::error::Error for Error {}
pub type Result<T> = std::result::Result<T, Error>;
pub fn fault(s: &'static str) -> Error {
    Error::Fault(s)
}
pub fn matches(pattern: &str, value: &str) -> bool {
    Regex::new(pattern).expect("constant regex").is_match(value)
}
pub const ENV_KEYS: &[&str] = &[
    "TV_HOST",
    "TV_LAUNCHER",
    "TV_CLIENT_KEY",
    "TV_CLIENT_KEY_FILE",
    "TV_SSH_KEY",
    "TV_SSH_PORT",
    "TV_SSH_USER",
    "TV_SSH_KNOWN_HOSTS",
    "TV_DEVMODE_TOKEN",
    "TVHOME_STATE_DIR",
    "HC_WATCHER_URL",
    "HC_RENEWAL_URL",
    "TV_WATCH_ENABLED",
    "TV_RENEW_ENABLED",
];

#[derive(Clone)]
pub struct Config {
    pub tv: String,
    pub values: BTreeMap<String, String>,
    pub home: PathBuf,
}
impl Config {
    pub fn validate_name(tv: &str) -> Result<()> {
        if !matches(r"^[a-z][a-z0-9-]{0,47}$", tv) {
            return Err(fault("TV name must start with a lowercase letter and contain only lowercase letters, digits or hyphens (48 characters maximum)."));
        }
        Ok(())
    }
    pub fn load(tv: &str, configure: bool) -> Result<Self> {
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .ok_or(fault("HOME is unavailable."))?;
        Self::load_at(home, tv, configure, std::env::vars())
    }
    pub fn load_at(
        home: PathBuf,
        tv: &str,
        configure: bool,
        environment: impl IntoIterator<Item = (String, String)>,
    ) -> Result<Self> {
        Self::validate_name(tv)?;
        let mut config = Self {
            tv: tv.into(),
            values: BTreeMap::new(),
            home,
        };
        if configure {
            // Only configure consumes environment variables. A saved TV can never
            // be redirected by another TV's variables left in the calling shell.
            config.values = environment
                .into_iter()
                .filter(|(k, _)| ENV_KEYS.contains(&k.as_str()))
                .collect();
        } else {
            config.values =
                Self::parse_snapshot(&fs::read(config.environment_path()).map_err(|_| {
                    fault("TV profile not found. Run tvhome --tv NAME setup first.")
                })?)?;
        }
        Ok(config)
    }
    pub fn profile_root(&self) -> PathBuf {
        self.home.join(".tvhome/tvs")
    }
    pub fn environment_path(&self) -> PathBuf {
        self.profile_root().join(&self.tv).join("environment.json")
    }
    pub fn installed_binary(&self) -> PathBuf {
        self.home
            .join(".tvhome/tvs")
            .join(&self.tv)
            .join("bin/tvhome")
    }
    pub fn catalog_dir(&self) -> PathBuf {
        PathBuf::from("catalog/tvs").join(&self.tv)
    }
    pub fn mac_label(&self, command: &str) -> String {
        format!("com.anant.tvhome.{}.{command}", self.tv)
    }
    pub fn linux_unit(&self, command: &str, kind: &str) -> String {
        format!("tvhome-{command}-{}.{kind}", self.tv)
    }
    pub fn check_unique(&self) -> Result<()> {
        let host = self.host()?;
        let urls = [
            self.ping_url("HC_WATCHER_URL", false)?,
            self.ping_url("HC_RENEWAL_URL", false)?,
        ];
        if !urls[0].is_empty() && urls[0] == urls[1] {
            return Err(fault(
                "Watcher and renewal need different Healthchecks URLs.",
            ));
        }
        if !self.profile_root().exists() {
            return Ok(());
        }
        for entry in
            fs::read_dir(self.profile_root()).map_err(|_| fault("Cannot read TV profiles."))?
        {
            let entry = entry.map_err(|_| fault("Cannot read TV profiles."))?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if name == self.tv || !entry.path().is_dir() {
                continue;
            }
            let other = Self::load_at(self.home.clone(), &name, false, [])?;
            if other.host()?.eq_ignore_ascii_case(host) {
                return Err(fault(
                    "This TV_HOST is already assigned to another profile. Use that TV name.",
                ));
            }
            for url in &urls {
                if !url.is_empty()
                    && [
                        other.ping_url("HC_WATCHER_URL", false)?,
                        other.ping_url("HC_RENEWAL_URL", false)?,
                    ]
                    .contains(url)
                {
                    return Err(fault("Healthchecks URLs must be unique across TVs so one TV cannot mask another's failure."));
                }
            }
            if self.key_file() == other.key_file() {
                return Err(fault("TV_CLIENT_KEY_FILE must be different for each TV."));
            }
            if let (Some(a), Some(b)) =
                (self.get("TV_DEVMODE_TOKEN"), other.get("TV_DEVMODE_TOKEN"))
            {
                if a == b {
                    return Err(fault(
                        "Developer Mode tokens must belong to one TV profile only.",
                    ));
                }
            }
        }
        Ok(())
    }
    pub fn configure(&self, replace: bool) -> Result<()> {
        let _lock = FileLock::acquire(&self.home.join(".tvhome/configure.lock"))?;
        self.host()?;
        self.launcher()?;
        self.check_unique()?;
        let mut operation_locks = Vec::new();
        if self.environment_path().exists() {
            if !replace {
                return Err(fault("Profile already exists. Use configure --replace to replace its complete configuration."));
            }
            let old = Self::load_at(self.home.clone(), &self.tv, false, [])?;
            for operation in ["watch", "renew", "setup", "catalog"] {
                operation_locks.push(FileLock::acquire(
                    &old.state_dir().join(format!("{operation}.lock")),
                )?);
            }
            if old.host()? != self.host()? {
                // An address may now point at a different TV: never reuse its renewal token.
                let token = old.state_dir().join("devmode-token");
                match fs::remove_file(token) {
                    Ok(()) => (),
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
                    Err(_) => {
                        return Err(fault(
                            "Cannot clear the token cache for the changed TV address.",
                        ))
                    }
                }
            }
        }
        private_write(
            &self.environment_path(),
            &serde_json::to_vec_pretty(&self.snapshot())
                .map_err(|_| fault("Cannot encode TV configuration."))?,
        )?;
        Ok(())
    }
    pub fn parse_snapshot(bytes: &[u8]) -> Result<BTreeMap<String, String>> {
        let values: BTreeMap<String, String> = serde_json::from_slice(bytes)
            .map_err(|_| fault("Invalid private environment file."))?;
        if values.keys().any(|k| !ENV_KEYS.contains(&k.as_str())) {
            return Err(fault("Invalid private environment file."));
        }
        Ok(values)
    }
    pub fn get(&self, key: &str) -> Option<&str> {
        self.values
            .get(key)
            .map(String::as_str)
            .filter(|s| !s.is_empty())
    }
    pub fn host(&self) -> Result<&str> {
        let host = self.get("TV_HOST").unwrap_or("");
        if !matches(r"^[A-Za-z0-9][A-Za-z0-9.-]{0,252}$", host) {
            return Err(fault("Set TV_HOST to the TV IPv4 address or hostname."));
        }
        Ok(host)
    }
    pub fn launcher(&self) -> Result<&str> {
        let value = self.get("TV_LAUNCHER").unwrap_or("com.anant.tvhome.native");
        if !matches(r"^[A-Za-z0-9][A-Za-z0-9._-]+$", value) {
            return Err(fault("TV_LAUNCHER must be an app ID."));
        }
        Ok(value)
    }
    pub fn path(&self, value: &str) -> PathBuf {
        if value == "~" {
            return self.home.clone();
        }
        if let Some(rest) = value.strip_prefix("~/") {
            return self.home.join(rest);
        }
        let p = PathBuf::from(value);
        if p.is_absolute() {
            p
        } else {
            std::env::current_dir()
                .unwrap_or_else(|_| self.home.clone())
                .join(p)
        }
    }
    pub fn state_dir(&self) -> PathBuf {
        self.path(self.get("TVHOME_STATE_DIR").unwrap_or("~/.tvhome"))
            .join("tvs")
            .join(&self.tv)
    }
    pub fn key_file(&self) -> PathBuf {
        self.get("TV_CLIENT_KEY_FILE")
            .map(|p| self.path(p))
            .unwrap_or_else(|| self.state_dir().join("pairing.json"))
    }
    pub fn key(&self) -> Option<String> {
        if let Some(value) = self.get("TV_CLIENT_KEY") {
            return Some(value.into());
        }
        let keys: serde_json::Value =
            serde_json::from_slice(&fs::read(self.key_file()).ok()?).ok()?;
        keys.get(self.host().ok()?)?
            .as_str()
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
    }
    pub fn save_key(&self, key: &str) -> Result<()> {
        let file = self.key_file();
        let mut keys: BTreeMap<String, String> = fs::read(&file)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        keys.insert(self.host()?.into(), key.into());
        private_write(
            &file,
            &serde_json::to_vec_pretty(&keys).map_err(|_| fault("Cannot encode pairing key."))?,
        )
    }
    pub fn ping_url(&self, key: &str, required: bool) -> Result<String> {
        let value = self.get(key).unwrap_or("").trim_end_matches('/');
        if value.is_empty() && !required {
            return Ok(String::new());
        }
        let invalid =
            fault("Set HC_WATCHER_URL and HC_RENEWAL_URL to base HTTPS hc-ping.com URLs.");
        let u = Url::parse(value).map_err(|_| invalid.clone())?;
        let uuid = r"[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}";
        let parts: Vec<_> = u.path().trim_matches('/').split('/').collect();
        let path_ok = matches(&format!("^/{uuid}$"), u.path())
            || (parts.len() == 2
                && !matches(&format!("^{uuid}$"), parts[0])
                && matches(r"^/[A-Za-z0-9_-]+/[A-Za-z0-9_-]+$", u.path()));
        if u.scheme() != "https"
            || u.host_str() != Some("hc-ping.com")
            || u.port_or_known_default() != Some(443)
            || !u.username().is_empty()
            || u.password().is_some()
            || u.query().is_some()
            || u.fragment().is_some()
            || !path_ok
        {
            return Err(invalid);
        }
        Ok(value.into())
    }
    pub fn snapshot(&self) -> BTreeMap<String, String> {
        let mut values = self.values.clone();
        values.retain(|k, _| ENV_KEYS.contains(&k.as_str()));
        for key in [
            "TV_CLIENT_KEY_FILE",
            "TV_SSH_KEY",
            "TV_SSH_KNOWN_HOSTS",
            "TVHOME_STATE_DIR",
        ] {
            if let Some(value) = self.get(key) {
                values.insert(key.into(), self.path(value).to_string_lossy().into());
            }
        }
        values
    }
}

pub struct FileLock(std::fs::File);
impl FileLock {
    pub fn acquire(path: &Path) -> Result<Self> {
        use std::os::{fd::AsRawFd, unix::fs::OpenOptionsExt};
        private_dir(path.parent().ok_or(fault("Invalid lock path."))?)?;
        let file = fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .mode(0o600)
            .open(path)
            .map_err(|_| fault("Cannot open tvhome lock."))?;
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            let error = std::io::Error::last_os_error();
            return Err(if error.kind() == std::io::ErrorKind::WouldBlock {
                Error::Busy
            } else {
                fault("Cannot acquire the operation lock.")
            });
        }
        Ok(Self(file))
    }
}
impl Drop for FileLock {
    fn drop(&mut self) {
        use std::os::fd::AsRawFd;
        unsafe {
            libc::flock(self.0.as_raw_fd(), libc::LOCK_UN);
        }
    }
}

pub fn private_dir(path: &Path) -> Result<()> {
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(path)
        .map_err(|_| fault("Cannot create private directory."))
}
pub fn private_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path.parent().ok_or(fault("Invalid private file path."))?;
    private_dir(parent)?;
    let mut file = tempfile::NamedTempFile::new_in(parent)
        .map_err(|_| fault("Cannot create private file."))?;
    file.as_file()
        .set_permissions(fs::Permissions::from_mode(0o600))
        .map_err(|_| fault("Cannot protect private file."))?;
    file.write_all(bytes)
        .map_err(|_| fault("Cannot write private file."))?;
    file.persist(path)
        .map_err(|_| fault("Cannot save private file."))?;
    Ok(())
}

#[derive(Clone)]
pub struct Http {
    client: Client,
}
impl Http {
    pub fn new() -> Result<Self> {
        Ok(Self {
            client: Client::builder()
                .redirect(Policy::none())
                .timeout(Duration::from_secs(10))
                .user_agent("tvhome/2")
                .build()
                .map_err(|_| fault("Cannot initialize HTTPS client."))?,
        })
    }
    pub async fn request(&self, url: &str, body: Option<&str>) -> Result<Vec<u8>> {
        let request = match body {
            Some(b) => self
                .client
                .post(url)
                .header("Content-Type", "text/plain")
                .body(b.to_owned()),
            None => self.client.get(url),
        };
        let response = request
            .send()
            .await
            .map_err(|_| fault("HTTPS request failed."))?;
        read_response(response, 65536).await
    }
    pub async fn ping(&self, url: &str, suffix: &str, message: &str) -> bool {
        if url.is_empty() {
            return true;
        }
        if self
            .request(&format!("{url}{suffix}"), Some(message))
            .await
            .is_err()
        {
            crate::log("Healthchecks ping failed; missing pings will be detected remotely.");
            return false;
        }
        true
    }
}
pub async fn read_response(mut response: reqwest::Response, limit: usize) -> Result<Vec<u8>> {
    if !response.status().is_success() {
        return Err(fault("HTTP service returned a non-success status."));
    }
    let mut data = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| fault("Could not read HTTP response."))?
    {
        if data.len() + chunk.len() > limit {
            return Err(fault("HTTP response exceeds its size limit."));
        }
        data.extend_from_slice(&chunk);
    }
    Ok(data)
}

#[cfg(test)]
pub fn test_config(root: &Path) -> Config {
    Config {
        tv: "living-room".into(),
        home: root.into(),
        values: [("TV_HOST", "tv.example"), ("TV_CLIENT_KEY", "fixture-key")]
            .into_iter()
            .map(|(k, v)| (k.into(), v.into()))
            .collect(),
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn http_does_not_follow_redirects_or_expose_request_secrets() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut bytes = [0; 4096];
            let count = socket.read(&mut bytes).await.unwrap();
            assert!(count > 0, "expected an HTTP request");
            socket.write_all(b"HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:1/secret\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await.unwrap();
        });
        let error = Http::new()
            .unwrap()
            .request(&format!("http://{address}/?token=privateToken"), None)
            .await
            .unwrap_err();
        server.await.unwrap();
        assert_eq!(error, fault("HTTP service returned a non-success status."));
        assert!(!error.to_string().contains("privateToken"));
    }
    #[test]
    fn configuration_rejects_host_injection() {
        let mut c = test_config(Path::new("/tmp"));
        for host in ["-oProxyCommand=x", "tv/a", "tv\nbad", "tv:3001", ""] {
            c.values.insert("TV_HOST".into(), host.into());
            assert!(c.host().is_err());
        }
        assert_eq!(c.launcher().unwrap(), "com.anant.tvhome.native");
    }
    #[test]
    fn health_urls_are_base_urls_and_errors_are_redacted() {
        let mut c = test_config(Path::new("/tmp"));
        for url in [
            "http://hc-ping.com/key/slug",
            "https://hc-ping.com/00000000-0000-0000-0000-000000000000/fail",
            "https://hc-ping.com/key/slug?secret=123",
            "https://hc-ping.com.evil.test/key/slug",
            "https://user:secret@hc-ping.com/key/slug",
        ] {
            c.values.insert("HC_WATCHER_URL".into(), url.into());
            assert!(!c
                .ping_url("HC_WATCHER_URL", true)
                .unwrap_err()
                .to_string()
                .contains("secret"));
        }
        c.values.insert(
            "HC_WATCHER_URL".into(),
            "https://hc-ping.com/project_key/home-watch/".into(),
        );
        assert_eq!(
            c.ping_url("HC_WATCHER_URL", true).unwrap(),
            "https://hc-ping.com/project_key/home-watch"
        );
    }
    #[test]
    fn keys_are_private_and_preserve_other_tvs() {
        let root = tempfile::tempdir().unwrap();
        let mut c = test_config(root.path());
        c.values.remove("TV_CLIENT_KEY");
        c.save_key("first").unwrap();
        c.values.insert("TV_HOST".into(), "second.example".into());
        c.save_key("second").unwrap();
        assert_eq!(c.key().unwrap(), "second");
        c.values.insert("TV_HOST".into(), "tv.example".into());
        assert_eq!(c.key().unwrap(), "first");
        assert_eq!(
            fs::metadata(c.key_file()).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            fs::metadata(c.state_dir()).unwrap().permissions().mode() & 0o777,
            0o700
        );
    }
    #[test]
    fn snapshots_only_accept_documented_variables() {
        assert!(Config::parse_snapshot(br#"{"PATH":"untrusted"}"#).is_err());
        assert!(Config::parse_snapshot(br#"{"TV_HOST":123}"#).is_err());
        assert!(Config::parse_snapshot(br#"{"TV_HOST":"tv.example"}"#).is_ok());
    }
}

#[cfg(test)]
mod multi_tv_tests {
    use super::*;
    fn profile(root: &Path, name: &str, host: &str) -> Config {
        Config::load_at(root.into(), name, true, [("TV_HOST".into(), host.into())]).unwrap()
    }
    #[test]
    fn saved_profiles_ignore_shell_variables_and_keep_credentials_separate() {
        let root = tempfile::tempdir().unwrap();
        let a = profile(root.path(), "living-room", "192.0.2.10");
        let b = profile(root.path(), "bedroom", "192.0.2.11");
        a.configure(false).unwrap();
        b.configure(false).unwrap();
        a.save_key("a-key").unwrap();
        b.save_key("b-key").unwrap();
        private_write(&a.state_dir().join("devmode-token"), b"a-token").unwrap();
        private_write(&b.state_dir().join("devmode-token"), b"b-token").unwrap();
        let loaded = Config::load_at(
            root.path().into(),
            "living-room",
            false,
            [
                ("TV_HOST".into(), "wrong-tv".into()),
                ("TV_CLIENT_KEY".into(), "wrong-key".into()),
            ],
        )
        .unwrap();
        assert_eq!(loaded.host().unwrap(), "192.0.2.10");
        assert_eq!(loaded.key().unwrap(), "a-key");
        assert_eq!(b.key().unwrap(), "b-key");
        assert_ne!(a.environment_path(), b.environment_path());
        assert_ne!(a.installed_binary(), b.installed_binary());
        assert_ne!(a.catalog_dir(), b.catalog_dir());
        assert_eq!(
            fs::read(b.state_dir().join("devmode-token")).unwrap(),
            b"b-token"
        );
        assert_eq!(
            fs::metadata(a.environment_path())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
    #[test]
    fn duplicate_tv_and_alert_routes_are_rejected() {
        let root = tempfile::tempdir().unwrap();
        let mut a = profile(root.path(), "living-room", "tv-one");
        a.values.insert(
            "HC_WATCHER_URL".into(),
            "https://hc-ping.com/key/one-watch".into(),
        );
        a.configure(false).unwrap();
        let mut b = profile(root.path(), "bedroom", "tv-one");
        assert!(b.configure(false).is_err());
        b.values.insert("TV_HOST".into(), "tv-two".into());
        b.values.insert(
            "HC_RENEWAL_URL".into(),
            "https://hc-ping.com/key/one-watch/".into(),
        );
        assert!(b.configure(false).is_err());
        b.values.insert(
            "HC_RENEWAL_URL".into(),
            "https://hc-ping.com/key/two-renew".into(),
        );
        b.configure(false).unwrap();
        assert!(
            b.configure(false).is_err(),
            "overwriting requires --replace"
        );
    }
    #[test]
    fn replacing_configuration_does_not_merge_secrets_or_reuse_an_old_address_token() {
        let root = tempfile::tempdir().unwrap();
        let mut a = profile(root.path(), "living-room", "tv-one");
        a.values
            .insert("TV_DEVMODE_TOKEN".into(), "old-token".into());
        a.configure(false).unwrap();
        private_write(&a.state_dir().join("devmode-token"), b"old-token").unwrap();
        let next = profile(root.path(), "living-room", "tv-new-address");
        next.configure(true).unwrap();
        let loaded = Config::load_at(root.path().into(), "living-room", false, []).unwrap();
        assert_eq!(loaded.host().unwrap(), "tv-new-address");
        assert!(loaded.get("TV_DEVMODE_TOKEN").is_none());
        assert!(!loaded.state_dir().join("devmode-token").exists());
    }
    #[test]
    fn running_tv_cannot_be_reconfigured_and_locks_are_per_tv() {
        let root = tempfile::tempdir().unwrap();
        let a = profile(root.path(), "living-room", "tv-one");
        let b = profile(root.path(), "bedroom", "tv-two");
        a.configure(false).unwrap();
        b.configure(false).unwrap();
        let guard = FileLock::acquire(&a.state_dir().join("watch.lock")).unwrap();
        assert!(FileLock::acquire(&a.state_dir().join("watch.lock")).is_err());
        assert!(FileLock::acquire(&b.state_dir().join("watch.lock")).is_ok());
        assert!(a.configure(true).is_err());
        b.configure(true).unwrap();
        drop(guard);
        a.configure(true).unwrap();
    }
    #[test]
    fn invalid_names_cannot_escape_profile_directories_or_create_service_syntax() {
        for name in [
            "",
            "../tv",
            "TV",
            "two TVs",
            "tv/other",
            "tv.service\n",
            "-tv",
        ] {
            assert!(Config::validate_name(name).is_err());
        }
        assert!(Config::validate_name(&"a".repeat(49)).is_err());
        assert!(Config::validate_name("living-room-2").is_ok());
    }
}
