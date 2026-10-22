use crate::{
    common::{fault, private_dir, private_write, read_response, Config, Error, FileLock, Result},
    install::UpdateJobs,
    inventory,
    package::{self, Package, APP_ID},
    self_install, setup,
    ssh::{self, TvSsh},
    tv,
};
use reqwest::{redirect::Policy, Client, Url};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};
use tokio::{
    process::Command,
    time::{sleep, Instant},
};

const REPO: &str = "anantc/webos-custom-home";
const APP: &str = "/media/developer/apps/usr/palm/applications/com.anant.tvhome.native";
const STATE_FILES: [&str; 2] = ["preferences.json", "speedtest-result.json"];

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Version([u64; 3]);
impl Version {
    fn parse(text: &str) -> Result<Self> {
        let parts: Vec<_> = text.split('.').collect();
        if parts.len() != 3
            || parts.iter().any(|p| {
                p.is_empty()
                    || !p.bytes().all(|b| b.is_ascii_digit())
                    || (p.len() > 1 && p.starts_with('0'))
            })
        {
            return Err(fault("Expected a release version in X.Y.Z format."));
        }
        let mut numbers = [0; 3];
        for (i, part) in parts.iter().enumerate() {
            numbers[i] = part
                .parse()
                .map_err(|_| fault("Invalid release version."))?;
        }
        Ok(Self(numbers))
    }
}
fn platform() -> Result<String> {
    let os = match std::env::consts::OS {
        "macos" => "darwin",
        "linux" => "linux",
        _ => return Err(fault("Updates support macOS and Linux.")),
    };
    let arch = match std::env::consts::ARCH {
        "aarch64" => "arm64",
        "x86_64" => "x64",
        _ => {
            return Err(fault(
                "No release is available for this computer architecture.",
            ))
        }
    };
    Ok(format!("tvhome-{os}-{arch}"))
}
struct Release {
    version: String,
    binary: String,
    checksum: String,
}
fn release(metadata: &Value, filename: &str) -> Result<Release> {
    if metadata["draft"] != false || metadata["prerelease"] != false {
        return Err(fault("GitHub did not return a published stable release."));
    }
    let tag = metadata["tag_name"]
        .as_str()
        .ok_or(fault("Release has no tag."))?;
    let version = tag.strip_prefix('v').ok_or(fault("Invalid release tag."))?;
    Version::parse(version)?;
    let base = format!("https://github.com/{REPO}/releases/download/{tag}/");
    let assets = metadata["assets"]
        .as_array()
        .ok_or(fault("Release has no downloads."))?;
    let asset = |name: &str| -> Result<String> {
        let expected = format!("{base}{name}");
        if assets
            .iter()
            .any(|a| a["name"] == name && a["browser_download_url"] == expected)
        {
            Ok(expected)
        } else {
            Err(fault("The latest release is missing this platform's executable or checksum. Retry once publication completes."))
        }
    };
    Ok(Release {
        version: version.into(),
        binary: asset(filename)?,
        checksum: asset(&format!("{filename}.sha256"))?,
    })
}
fn trusted_url(url: &Url) -> bool {
    url.scheme() == "https"
        && url.username().is_empty()
        && url.password().is_none()
        && url.port_or_known_default() == Some(443)
        && matches!(
            url.host_str(),
            Some(
                "api.github.com"
                    | "github.com"
                    | "release-assets.githubusercontent.com"
                    | "objects.githubusercontent.com"
            )
        )
}
fn client() -> Result<Client> {
    Client::builder()
        .https_only(true)
        .user_agent(concat!("tvhome/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(180))
        .redirect(Policy::custom(|attempt| {
            if attempt.previous().len() < 5 && trusted_url(attempt.url()) {
                attempt.follow()
            } else {
                attempt.error("Untrusted release redirect")
            }
        }))
        .build()
        .map_err(|_| fault("Cannot initialize release downloads."))
}
async fn download(client: &Client, url: &str, limit: usize) -> Result<Vec<u8>> {
    if !trusted_url(&Url::parse(url).map_err(|_| fault("Invalid release URL."))?) {
        return Err(fault("Untrusted release URL."));
    }
    let response = client.get(url).send().await.map_err(|_| {
        fault("Cannot download the release from GitHub. Check this computer's internet connection.")
    })?;
    if !response.status().is_success() {
        return Err(fault("GitHub could not serve the release. It may be temporarily unavailable or rate limited; retry later."));
    }
    read_response(response, limit).await
}
fn verify(bytes: &[u8], checksum: &[u8], filename: &str) -> Result<()> {
    let text =
        std::str::from_utf8(checksum).map_err(|_| fault("Invalid release checksum file."))?;
    let parts: Vec<_> = text.split_whitespace().collect();
    if parts.len() != 2
        || parts[1].trim_start_matches('*') != filename
        || parts[0].len() != 64
        || !parts[0].bytes().all(|b| b.is_ascii_hexdigit())
        || parts[0].to_ascii_lowercase() != format!("{:x}", Sha256::digest(bytes))
    {
        return Err(fault(
            "Release checksum verification failed. Nothing has been installed.",
        ));
    }
    Ok(())
}

pub async fn run(selected: Option<&str>, apply: bool) -> Result<()> {
    let home = PathBuf::from(std::env::var_os("HOME").ok_or(fault("HOME is unavailable."))?);
    if !home.is_absolute() {
        return Err(fault("HOME must be an absolute path."));
    }
    let names = if let Some(name) = selected {
        Config::load_at(home.clone(), name, false, [])?;
        vec![name.to_owned()]
    } else {
        inventory::profile_names(&home)?
    };
    if names.is_empty() {
        println!("No saved TVs. Run tvhome --tv living-room setup first.");
        return Ok(());
    }
    let stop = Arc::new(AtomicBool::new(false));
    let signal_stop = stop.clone();
    let signals = tokio::spawn(async move {
        while tokio::signal::ctrl_c().await.is_ok() {
            signal_stop.store(true, Ordering::Relaxed);
            eprintln!("Stopping after the current TV finishes and its background jobs restart.");
        }
    });
    let result = if apply {
        apply_all(&home, &names, &stop).await
    } else {
        fetch_and_apply(&home, &names, selected, &stop).await
    };
    signals.abort();
    result
}
async fn fetch_and_apply(
    home: &Path,
    names: &[String],
    selected: Option<&str>,
    stop: &AtomicBool,
) -> Result<()> {
    let _lock = FileLock::acquire(&home.join(".tvhome/update.lock"))?;
    println!("Checking the latest stable tvhome release.");
    let client = client()?;
    let metadata = download(
        &client,
        &format!("https://api.github.com/repos/{REPO}/releases/latest"),
        1024 * 1024,
    )
    .await?;
    let metadata: Value =
        serde_json::from_slice(&metadata).map_err(|_| fault("Invalid GitHub release response."))?;
    let filename = platform()?;
    let release = release(&metadata, &filename)?;
    match Version::parse(&release.version)?.cmp(&Version::parse(env!("CARGO_PKG_VERSION"))?) {
        std::cmp::Ordering::Less => return Err(fault("This tvhome build is newer than the latest published release. Update will not downgrade it.")),
        std::cmp::Ordering::Equal => {
            println!("tvhome {} is current; checking each TV separately.", release.version);
            return apply_all(home, names, stop).await;
        }
        std::cmp::Ordering::Greater => (),
    }
    println!("Downloading tvhome {} for this computer.", release.version);
    let checksum = download(&client, &release.checksum, 4096).await?;
    let bytes = download(&client, &release.binary, 128 * 1024 * 1024).await?;
    verify(&bytes, &checksum, &filename)?;
    if stop.load(Ordering::Relaxed) {
        return Err(fault("Update interrupted before installation."));
    }
    let downloads = home.join(".tvhome/downloads");
    private_dir(&downloads)?;
    let stage = tempfile::tempdir_in(downloads).map_err(|_| fault("Cannot stage the release."))?;
    let executable = stage.path().join("tvhome");
    private_write(&executable, &bytes)?;
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o700))
        .map_err(|_| fault("Cannot mark the update executable."))?;
    let output = tokio::time::timeout(
        Duration::from_secs(15),
        Command::new(&executable)
            .arg("--version")
            .kill_on_drop(true)
            .output(),
    )
    .await
    .map_err(|_| fault("Updated executable did not respond."))?
    .map_err(|_| fault("Cannot run the verified update on this computer."))?;
    if !output.status.success()
        || String::from_utf8_lossy(&output.stdout).trim() != format!("tvhome {}", release.version)
    {
        return Err(fault(
            "Verified executable does not report the expected release version.",
        ));
    }
    if stop.load(Ordering::Relaxed) {
        return Err(fault("Update interrupted before installation."));
    }
    // The new executable supplies its own embedded Home package and license notices.
    let mut child = Command::new(&executable);
    if let Some(name) = selected {
        child.args(["--tv", name]);
    }
    let result = child
        .args(["update", "--apply"])
        .status()
        .await
        .map_err(|_| fault("Cannot start the verified update."))?;
    if result.success() {
        Ok(())
    } else {
        Err(fault("Update did not complete for every TV. See the per-TV results above and rerun tvhome update."))
    }
}

#[derive(Debug, PartialEq, Eq)]
enum Outcome {
    Updated,
    Current,
    Offline,
    Newer,
}
trait Updater {
    async fn one(&self, name: &str) -> Result<Outcome>;
}
#[derive(Default, Debug, PartialEq, Eq)]
struct Summary {
    updated: usize,
    current: usize,
    skipped: usize,
    failed: usize,
}
async fn fleet(updater: &impl Updater, names: &[String], stop: &AtomicBool) -> Summary {
    let mut summary = Summary::default();
    for name in names {
        if stop.load(Ordering::Relaxed) {
            break;
        }
        println!("Checking {name}.");
        match updater.one(name).await {
            Ok(Outcome::Updated) => {
                summary.updated += 1;
                println!("{name}: updated.");
            }
            Ok(Outcome::Current) => {
                summary.current += 1;
                println!("{name}: Home is current; background executable checked.");
            }
            Ok(Outcome::Offline) | Err(Error::Unavailable) => {
                summary.skipped += 1;
                println!(
                    "{name}: off or unreachable — skipped. Turn it on and rerun tvhome update."
                );
            }
            Ok(Outcome::Newer) => {
                summary.skipped += 1;
                println!("{name}: Home is newer than the release — left unchanged.");
            }
            Err(error) => {
                summary.failed += 1;
                println!("{name}: failed: {error}");
            }
        }
    }
    summary
}
async fn apply_all(home: &Path, names: &[String], stop: &AtomicBool) -> Result<()> {
    let package = Package::read(package::bundled())?;
    package.policy()?;
    let version = package.version()?;
    Version::parse(&version)?;
    if stop.load(Ordering::Relaxed) {
        return Err(fault("Update interrupted before installation."));
    }
    // Linux current_exe points at a deleted inode after replacing the running file.
    // Keep the bytes for per-TV jobs before performing that atomic replacement.
    let executable =
        std::env::current_exe().map_err(|_| fault("Cannot locate this executable."))?;
    let binary = fs::read(executable).map_err(|_| fault("Cannot read this executable."))?;
    self_install::update(home)?;
    println!(
        "tvhome {} installed. Checking {} TV(s) for Home {version}.",
        env!("CARGO_PKG_VERSION"),
        names.len()
    );
    let result = fleet(
        &SystemUpdater {
            home,
            version: &version,
            binary: &binary,
        },
        names,
        stop,
    )
    .await;
    println!(
        "{} updated, {} current, {} skipped, {} failed.",
        result.updated, result.current, result.skipped, result.failed
    );
    if cfg!(target_os = "macos") {
        println!("If macOS asks again, allow tvhome Local Network access for its background jobs.");
    }
    if stop.load(Ordering::Relaxed) {
        return Err(fault(
            "Update interrupted; rerun to check the remaining TVs.",
        ));
    }
    if result.failed > 0 {
        Err(fault(
            "Some TVs could not be updated. Fix the reported issue and rerun tvhome update.",
        ))
    } else {
        Ok(())
    }
}
struct SystemUpdater<'a> {
    home: &'a Path,
    version: &'a str,
    binary: &'a [u8],
}
impl Updater for SystemUpdater<'_> {
    async fn one(&self, name: &str) -> Result<Outcome> {
        let config = Config::load_at(self.home.to_owned(), name, false, [])?;
        let _background = FileLock::acquire(&config.state_dir().join("background.lock"))?;
        let _setup = FileLock::acquire(&config.state_dir().join("setup.lock"))?;
        // Read power state before SSH/install: Quick Start+ can leave ports open in standby.
        let (points, inputs) = match capture_awake(&config).await {
            Ok(catalog) => catalog,
            Err(Error::Unavailable) => return Ok(Outcome::Offline),
            Err(error) => return Err(error),
        };
        let worker = config.clone();
        let installed = tokio::task::spawn_blocking(move || {
            installed_version(&TvSsh::connect(&worker, false)?)
        })
        .await
        .map_err(|_| fault("TV version check failed."))??;
        let outcome = match installed.as_deref().map(Version::parse).transpose()? {
            Some(v) if v > Version::parse(self.version)? => return Ok(Outcome::Newer),
            Some(v) if v == Version::parse(self.version)? => Outcome::Current,
            _ => Outcome::Updated,
        };
        let package = Package::read(package::bundled())?;
        let catalog = if outcome == Outcome::Updated {
            Some(package.catalog(&config, &points, &inputs).await?)
        } else {
            None
        };
        let jobs = UpdateJobs::capture(&config).await?;
        let controller = JobControl {
            jobs: &jobs,
            config: &config,
        };
        paused(&controller, async {
            let _locks = wait_for_jobs(&config).await?;
            if let Some(catalog) = catalog {
                let worker = config.clone();
                let version = self.version.to_owned();
                tokio::task::spawn_blocking(move || {
                    install_update(&worker, package, &catalog, &version)
                })
                .await
                .map_err(|_| fault("TV update task failed."))??;
            }
            inventory::record(&config, &points, &inputs)?;
            jobs.refresh_binary(&config, self.binary)?;
            Ok(outcome)
        })
        .await
    }
}
fn awake(power: &Value) -> Result<bool> {
    match power["state"]
        .as_str()
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("active") => Ok(true),
        Some("active standby" | "suspend" | "power off" | "screen off") => Ok(false),
        _ => Err(fault(
            "Cannot confirm the TV is awake. Turn it on and retry.",
        )),
    }
}
async fn capture_awake(config: &Config) -> Result<(Vec<Value>, Vec<Value>)> {
    let mut peer = tv::connect(config, false).await?;
    let result = async {
        if !awake(
            &peer
                .request(
                    "ssap://com.webos.service.tvpower/power/getPowerState",
                    "power",
                )
                .await?,
        )? {
            return Err(Error::Unavailable);
        }
        let apps = peer
            .request(
                "ssap://com.webos.applicationManager/listLaunchPoints",
                "apps",
            )
            .await?;
        let inputs = peer
            .request("ssap://tv/getExternalInputList", "inputs")
            .await?;
        Ok((
            apps["launchPoints"]
                .as_array()
                .ok_or(fault("TV did not return its apps."))?
                .clone(),
            inputs["devices"]
                .as_array()
                .ok_or(fault("TV did not return its inputs."))?
                .clone(),
        ))
    }
    .await;
    peer.close().await;
    result
}
trait JobPause {
    async fn pause(&self) -> Result<()>;
    async fn resume(&self) -> Result<()>;
}
struct JobControl<'a> {
    jobs: &'a UpdateJobs,
    config: &'a Config,
}
impl JobPause for JobControl<'_> {
    async fn pause(&self) -> Result<()> {
        self.jobs.pause().await
    }
    async fn resume(&self) -> Result<()> {
        self.jobs.resume(self.config).await
    }
}
async fn paused<T>(
    jobs: &impl JobPause,
    operation: impl std::future::Future<Output = Result<T>>,
) -> Result<T> {
    let result = match jobs.pause().await {
        Ok(()) => operation.await,
        Err(error) => Err(error),
    };
    let resumed = jobs.resume().await;
    if let Err(error) = &result {
        crate::log(&format!("TV update failed: {error}"));
    }
    resumed?;
    result
}
async fn wait_for_jobs(config: &Config) -> Result<Vec<FileLock>> {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let locks = ["watch", "renew", "catalog"]
            .iter()
            .map(|kind| FileLock::acquire(&config.state_dir().join(format!("{kind}.lock"))))
            .collect::<Result<Vec<_>>>();
        match locks {
            Err(Error::Busy) if Instant::now() < deadline => {
                sleep(Duration::from_millis(100)).await
            }
            result => return result,
        }
    }
}
fn installed_version(connection: &TvSsh) -> Result<Option<String>> {
    let path = ssh::quote(&format!("{APP}/appinfo.json"));
    let bytes = connection.command(&format!("if test -e {path}; then cat {path}; fi"))?;
    if bytes.is_empty() {
        return Ok(None);
    }
    let info: Value =
        serde_json::from_slice(&bytes).map_err(|_| fault("Invalid installed Home metadata."))?;
    if info["id"] != APP_ID {
        return Err(fault("Unexpected installed Home identity."));
    }
    Ok(Some(
        info["version"]
            .as_str()
            .ok_or(fault("Installed Home has no version."))?
            .to_owned(),
    ))
}
fn close_home(connection: &TvSsh) -> Result<()> {
    let running =
        || connection.command("if pidof tvhome-native >/dev/null; then printf running; fi");
    if running()?.is_empty() {
        return Ok(());
    }
    connection.luna(
        "luna://com.webos.applicationManager/dev/closeByAppId",
        &json!({"id":APP_ID}),
        None,
    )?;
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while !running()?.is_empty() {
        if std::time::Instant::now() >= deadline {
            return Err(fault(
                "Home did not close; update stopped to protect its settings.",
            ));
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    Ok(())
}
fn read_state(connection: &TvSsh) -> Result<BTreeMap<String, Vec<u8>>> {
    let mut files = BTreeMap::new();
    for name in STATE_FILES {
        let path = ssh::quote(&format!("{APP}/state/{name}"));
        // Prefix distinguishes a missing file from an empty/unreadable saved file.
        let bytes = connection.command(&format!(
            "if test -e {path}; then printf present; cat {path}; fi"
        ))?;
        if bytes.is_empty() {
            continue;
        }
        let data = bytes
            .strip_prefix(b"present")
            .ok_or(fault("Invalid Home settings reply."))?;
        serde_json::from_slice::<Value>(data).map_err(|_| {
            fault("Home has an invalid settings file. Update stopped before installation.")
        })?;
        files.insert(name.to_owned(), data.to_vec());
    }
    Ok(files)
}
fn backup_state(config: &Config, files: &BTreeMap<String, Vec<u8>>) -> Result<PathBuf> {
    let root = config.state_dir().join("update-backups");
    private_dir(&root)?;
    let backup = tempfile::Builder::new()
        .prefix("home-")
        .tempdir_in(root)
        .map_err(|_| fault("Cannot create Home settings backup."))?;
    for (name, bytes) in files {
        private_write(&backup.path().join(name), bytes)?;
    }
    private_write(
        &backup.path().join("tv.json"),
        json!({"tv":config.tv,"host":config.host()?})
            .to_string()
            .as_bytes(),
    )?;
    Ok(backup.keep())
}

fn install_update(
    config: &Config,
    mut package: Package,
    catalog: &package::CatalogBundle,
    version: &str,
) -> Result<()> {
    let connection = TvSsh::connect(config, false)?;
    close_home(&connection)?;
    let saved = read_state(&connection)?;
    let backup = backup_state(config, &saved)?;
    println!("{}: saved Home settings to {}", config.tv, backup.display());
    package.preserve_state(&saved)?;
    let ipk = package.personalize(catalog)?;
    setup::install_package(&connection, &ipk)?;
    if installed_version(&connection)?.as_deref() != Some(version) {
        return Err(fault(
            "Home did not report the expected version after installation.",
        ));
    }
    if read_state(&connection)? != saved {
        return Err(fault("Home settings verification failed. The pre-update copy is in this TV's update-backups folder."));
    }
    connection.luna(
        "luna://com.webos.applicationManager/launch",
        &json!({"id":APP_ID}),
        None,
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::RefCell, collections::VecDeque};

    #[cfg(feature = "bundled-home")]
    #[tokio::test]
    #[ignore = "run through tests/ssh-fixture.cjs"]
    async fn real_ssh_fixture_update_preserves_state() {
        let dir = PathBuf::from(std::env::var_os("TVHOME_SSH_FIXTURE").unwrap());
        let home = tempfile::tempdir().unwrap();
        let mut config = crate::common::test_config(home.path());
        config.values.insert("TV_HOST".into(), "127.0.0.1".into());
        for (key, file) in [("TV_SSH_KEY", "key"), ("TV_SSH_KNOWN_HOSTS", "known_hosts")] {
            config
                .values
                .insert(key.into(), dir.join(file).to_str().unwrap().into());
        }
        config.values.insert(
            "TV_SSH_PORT".into(),
            fs::read_to_string(dir.join("port")).unwrap(),
        );
        let connection = TvSsh::connect(&config, false).unwrap();
        connection.command("fixture-reset-home").unwrap();
        let saved = read_state(&connection).unwrap();
        assert_eq!(saved.len(), 2);
        assert_eq!(
            installed_version(&connection).unwrap().as_deref(),
            Some("0.0.1")
        );
        let package = Package::read(package::bundled()).unwrap();
        let version = package.version().unwrap();
        let catalog = package
            .catalog(&config, &[json!({"id":"netflix","title":"Netflix"})], &[])
            .await
            .unwrap();
        install_update(&config, package, &catalog, &version).unwrap();
        assert_eq!(read_state(&connection).unwrap(), saved);
        assert_eq!(
            installed_version(&connection).unwrap().as_deref(),
            Some(version.as_str())
        );
        assert_eq!(
            connection.command("fixture-home-running").unwrap(),
            b"running"
        );
        let backup = fs::read_dir(config.state_dir().join("update-backups"))
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        assert_eq!(
            fs::read(backup.join("preferences.json")).unwrap(),
            saved["preferences.json"]
        );

        connection.command("fixture-fail-next-install").unwrap();
        let package = Package::read(package::bundled()).unwrap();
        assert!(install_update(&config, package, &catalog, &version).is_err());
        assert_eq!(read_state(&connection).unwrap(), saved);
    }

    #[test]
    fn versions_sort_numerically_and_reject_tags_or_prereleases() {
        assert!(Version::parse("0.1.10").unwrap() > Version::parse("0.1.9").unwrap());
        for invalid in [
            "v0.1.5",
            "0.1.5-rc1",
            "0.1",
            "1.2.3.4",
            "01.2.3",
            "1.2.+3",
            "1.2.9999999999999999999999999999999",
        ] {
            assert!(Version::parse(invalid).is_err(), "{invalid}");
        }
    }
    #[test]
    fn release_and_checksum_must_match_the_official_asset() {
        let name = "tvhome-darwin-arm64";
        let base = format!("https://github.com/{REPO}/releases/download/v0.1.9/");
        let mut metadata = json!({"draft":false,"prerelease":false,"tag_name":"v0.1.9","assets":[
            {"name":name,"browser_download_url":format!("{base}{name}")},
            {"name":format!("{name}.sha256"),"browser_download_url":format!("{base}{name}.sha256")}
        ]});
        assert_eq!(release(&metadata, name).unwrap().version, "0.1.9");
        metadata["assets"][0]["browser_download_url"] = "https://example.org/tvhome".into();
        assert!(release(&metadata, name).is_err());
        metadata["prerelease"] = true.into();
        assert!(release(&metadata, name).is_err());
        let bytes = b"verified executable";
        let checksum = format!("{:x}  {name}\n", Sha256::digest(bytes));
        verify(bytes, checksum.as_bytes(), name).unwrap();
        assert!(verify(b"damaged", checksum.as_bytes(), name).is_err());
        assert!(verify(bytes, checksum.as_bytes(), "tvhome-linux-x64").is_err());
        assert!(verify(bytes, format!("{checksum}{checksum}").as_bytes(), name).is_err());
        for url in [
            "http://github.com/file",
            "https://github.com.evil.test/file",
            "https://other.githubusercontent.com/file",
            "https://github.com:8443/file",
            "https://user@github.com/file",
        ] {
            assert!(!trusted_url(&Url::parse(url).unwrap()), "{url}");
        }
        assert!(trusted_url(
            &Url::parse("https://release-assets.githubusercontent.com/path?token=signed").unwrap()
        ));
    }
    #[test]
    fn standby_ports_are_not_treated_as_awake_and_unknown_is_an_error() {
        assert!(awake(&json!({"state":"Active"})).unwrap());
        for state in ["Active Standby", "Suspend", "Power Off", "Screen Off"] {
            assert!(!awake(&json!({"state":state})).unwrap());
        }
        assert!(awake(&json!({})).is_err());
        assert!(awake(&json!({"state":"Unknown"})).is_err());
    }
    struct FakeUpdater {
        outcomes: RefCell<VecDeque<Result<Outcome>>>,
        visited: RefCell<Vec<String>>,
    }
    impl Updater for FakeUpdater {
        async fn one(&self, name: &str) -> Result<Outcome> {
            self.visited.borrow_mut().push(name.into());
            self.outcomes.borrow_mut().pop_front().unwrap()
        }
    }
    #[tokio::test]
    async fn fleet_continues_after_offline_failure_and_current_tvs() {
        let updater = FakeUpdater {
            outcomes: RefCell::new(VecDeque::from([
                Ok(Outcome::Current),
                Err(Error::Unavailable),
                Err(fault("SSH key rejected")),
                Ok(Outcome::Updated),
                Ok(Outcome::Newer),
            ])),
            visited: RefCell::new(Vec::new()),
        };
        let names = ["office", "guestroom", "bedroom", "living-room", "test"].map(str::to_owned);
        assert_eq!(
            fleet(&updater, &names, &AtomicBool::new(false)).await,
            Summary {
                current: 1,
                skipped: 2,
                failed: 1,
                updated: 1
            }
        );
        assert_eq!(*updater.visited.borrow(), names);
    }
    #[tokio::test]
    async fn cancelled_fleet_does_not_start_another_tv() {
        let updater = FakeUpdater {
            outcomes: RefCell::new(VecDeque::new()),
            visited: RefCell::new(Vec::new()),
        };
        let result = fleet(&updater, &["office".into()], &AtomicBool::new(true)).await;
        assert_eq!(result, Summary::default());
        assert!(updater.visited.borrow().is_empty());
    }
    struct FakeJobs {
        events: RefCell<Vec<&'static str>>,
        pause_fails: bool,
        resume_fails: bool,
    }
    impl JobPause for FakeJobs {
        async fn pause(&self) -> Result<()> {
            self.events.borrow_mut().push("pause");
            if self.pause_fails {
                Err(fault("partially stopped"))
            } else {
                Ok(())
            }
        }
        async fn resume(&self) -> Result<()> {
            self.events.borrow_mut().push("resume");
            if self.resume_fails {
                Err(fault("restart failed"))
            } else {
                Ok(())
            }
        }
    }
    #[tokio::test]
    async fn jobs_resume_even_after_partial_pause_or_install_failure() {
        for pause_fails in [false, true] {
            let jobs = FakeJobs {
                events: RefCell::new(Vec::new()),
                pause_fails,
                resume_fails: false,
            };
            let result: Result<()> = paused(&jobs, async {
                jobs.events.borrow_mut().push("install");
                Err(fault("install failed"))
            })
            .await;
            assert!(result.is_err());
            let expected = if pause_fails {
                vec!["pause", "resume"]
            } else {
                vec!["pause", "install", "resume"]
            };
            assert_eq!(*jobs.events.borrow(), expected);
        }
    }
    #[tokio::test]
    async fn successful_install_with_failed_restart_is_reported_as_failure() {
        let jobs = FakeJobs {
            events: RefCell::new(Vec::new()),
            pause_fails: false,
            resume_fails: true,
        };
        assert_eq!(
            paused(&jobs, async { Ok(()) }).await,
            Err(fault("restart failed"))
        );
    }
    #[test]
    fn saved_profiles_and_backups_remain_separate() {
        let home = tempfile::tempdir().unwrap();
        for tv in ["office", "guestroom"] {
            let path = home.path().join(".tvhome/tvs").join(tv);
            fs::create_dir_all(&path).unwrap();
            fs::write(path.join("environment.json"), "{}").unwrap();
        }
        fs::create_dir_all(home.path().join(".tvhome/tvs/unconfigured")).unwrap();
        assert_eq!(
            inventory::profile_names(home.path()).unwrap(),
            vec!["guestroom", "office"]
        );
        for tv in ["office", "guestroom"] {
            let mut config = crate::common::test_config(home.path());
            config.tv = tv.into();
            let bytes = format!("{{\"city\":\"{tv}\"}}").into_bytes();
            let files = BTreeMap::from([("preferences.json".into(), bytes.clone())]);
            let path = backup_state(&config, &files).unwrap();
            assert!(path.starts_with(config.state_dir()));
            assert_eq!(fs::read(path.join("preferences.json")).unwrap(), bytes);
            assert_eq!(
                fs::metadata(path.join("preferences.json"))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
    }
}
