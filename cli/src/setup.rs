use crate::{
    common::{fault, private_write, read_response, Config, FileLock, Result},
    package::{self, Package, APP_ID},
    ssh::{self, TvSsh},
};
use openssl::pkey::PKey;
use reqwest::{redirect::Policy, Client};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{self, Write},
    path::Path,
    time::Duration,
};

pub fn prompt(message: &str) -> Result<String> {
    print!("{message}");
    io::stdout()
        .flush()
        .map_err(|_| fault("Cannot display setup prompt."))?;
    let mut value = String::new();
    if io::stdin()
        .read_line(&mut value)
        .map_err(|_| fault("Cannot read setup input."))?
        == 0
    {
        return Err(fault("Setup needs an interactive terminal."));
    }
    Ok(value.trim().to_owned())
}
pub fn confirm(message: &str) -> Result<bool> {
    Ok(matches!(
        prompt(message)?.to_ascii_lowercase().as_str(),
        "y" | "yes"
    ))
}
fn unlock(bytes: &[u8], passphrase: &str) -> Result<Vec<u8>> {
    let key =
        PKey::private_key_from_pem_passphrase(bytes, passphrase.as_bytes()).map_err(|_| {
            fault("Cannot unlock the Developer Mode key. Check the displayed passphrase.")
        })?;
    key.private_key_to_pem_pkcs8()
        .map_err(|_| fault("Cannot save the Developer Mode key."))
}
async fn fetch_key(config: &Config) -> Result<Vec<u8>> {
    let client = Client::builder()
        .no_proxy()
        .redirect(Policy::none())
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|_| fault("Cannot initialize Developer Mode key download."))?;
    read_response(
        client
            .get(format!("http://{}:9991/webos_rsa", config.host()?))
            .send()
            .await
            .map_err(|_| {
                fault("Cannot retrieve the TV key. Turn on Key Server in Developer Mode and retry.")
            })?,
        65536,
    )
    .await
}
pub async fn run(
    tv: &str,
    host: Option<&str>,
    refresh_key: bool,
    package_path: Option<&Path>,
) -> Result<()> {
    Config::validate_name(tv)?;
    let home = std::env::var_os("HOME").ok_or(fault("HOME is unavailable."))?;
    let empty = Config::load_at(home.into(), tv, true, [])?;
    let existing = empty.environment_path().exists();
    let mut config = if existing {
        Config::load(tv, false)?
    } else {
        empty
    };
    if let Some(host) = host {
        config.values.insert("TV_HOST".into(), host.into());
    }
    if config.get("TV_HOST").is_none() {
        config.values.insert(
            "TV_HOST".into(),
            prompt("TV IP address shown in Developer Mode: ")?,
        );
    }
    config.host()?;
    // Validate the payload before recording credentials or contacting the TV.
    let bytes = if let Some(path) = package_path {
        fs::read(path).map_err(|_| fault("Cannot read the supplied Home template."))?
    } else {
        package::bundled().to_vec()
    };
    let package = Package::read(&bytes)?;
    package.policy()?;
    config.configure(existing)?;
    let _lock = FileLock::acquire(&config.state_dir().join("setup.lock"))?;
    println!("Setting up {} at {}.", config.tv, config.host()?);
    println!("On the TV, enable Developer Mode and Key Server. Developer Mode grants installation access; Key Server supplies this computer's SSH key.");
    let key = ssh::key_path(&config);
    if refresh_key || !key.exists() {
        prompt("Press Return when the TV is ready: ")?;
        let encrypted = fetch_key(&config).await?;
        let passphrase = rpassword::prompt_password("Passphrase shown in Developer Mode: ")
            .map_err(|_| fault("Cannot read the Developer Mode passphrase."))?;
        let decrypted = unlock(&encrypted, &passphrase)?;
        private_write(&key, &decrypted)?;
    }
    let worker = config.clone();
    tokio::task::spawn_blocking(move || TvSsh::connect(&worker, true)?.command("true"))
        .await
        .map_err(|_| fault("TV access check failed."))??;
    println!("SSH access verified. Approve the remote-control pairing prompt on the TV so setup can read its app list.");
    let (points, inputs) = crate::sync::capture(&config, true).await?;
    println!("Preparing Home for this TV's installed apps.");
    let catalog = package.catalog(&config, &points, &inputs).await?;
    let ipk = package.personalize(&catalog)?;
    let worker = config.clone();
    tokio::task::spawn_blocking(move || install_app(&worker, &ipk))
        .await
        .map_err(|_| fault("TV installation task failed."))??;
    crate::inventory::record(&config, &points, &inputs)?;
    println!("Home is installed and open on {}.", config.tv);
    println!(
        "To choose optional Home watching, session renewal and alerts, run: tvhome --tv {} background",
        config.tv
    );
    Ok(())
}
pub fn install_app(config: &Config, bytes: &[u8]) -> Result<()> {
    crate::update::install_prepared(config, Package::read(bytes)?)
}

pub(crate) fn install_package(connection: &TvSsh, bytes: &[u8]) -> Result<()> {
    let id = SystemNonce::generate()?;
    let path = format!("/media/developer/temp/tvhome-{id}.ipk");
    connection.command("mkdir -p /media/developer/temp")?;
    let result = (|| {
        connection.upload(&path, bytes)?;
        let digest = connection.command(&format!("sha256sum {}", ssh::quote(&path)))?;
        let received = std::str::from_utf8(&digest)
            .map_err(|_| fault("Invalid TV package checksum."))?
            .split_whitespace()
            .next()
            .unwrap_or("");
        let expected = format!("{:x}", Sha256::digest(bytes));
        if received != expected {
            return Err(fault("Uploaded package checksum does not match."));
        }
        println!("Installing Home on the TV.");
        connection.luna(
            "luna://com.webos.appInstallService/dev/install",
            &json!({"id":APP_ID,"ipkUrl":path,"subscribe":true}),
            Some("INSTALLED"),
        )?;
        let apps = connection.luna(
            "luna://com.webos.applicationManager/dev/listApps",
            &json!({"subscribe":false}),
            None,
        )?;
        if !apps["apps"]
            .as_array()
            .is_some_and(|apps| apps.iter().any(|a| a["id"] == APP_ID))
        {
            return Err(fault(
                "TV reported installation success but Home is missing from its app list.",
            ));
        }
        Ok(())
    })();
    let _ = connection.command(&format!("rm -f {}", ssh::quote(&path)));
    result
}
struct SystemNonce;
impl SystemNonce {
    fn generate() -> Result<String> {
        // Only hexadecimal bytes become part of a remote path.
        let mut random = [0u8; 16];
        openssl::rand::rand_bytes(&mut random)
            .map_err(|_| fault("Cannot generate an upload identifier."))?;
        Ok(random.iter().map(|b| format!("{b:02x}")).collect())
    }
}
pub async fn remove(config: &Config) -> Result<()> {
    if !confirm(&format!("Remove Home from {}? [y/N] ", config.tv))? {
        return Ok(());
    }
    let config = config.clone();
    tokio::task::spawn_blocking(move || {
        TvSsh::connect(&config, false)?.luna(
            "luna://com.webos.appInstallService/dev/remove",
            &json!({"id":APP_ID,"subscribe":true}),
            Some("REMOVED"),
        )?;
        Ok(())
    })
    .await
    .map_err(|_| fault("TV removal task failed."))?
}
fn prompt_ping_url(
    config: &Config,
    key: &str,
    label: &str,
    mut ask: impl FnMut(&str) -> Result<String>,
) -> Result<String> {
    let saved = config
        .ping_url(key, false)
        .ok()
        .filter(|url| !url.is_empty());
    let message = if saved.is_some() {
        format!("{label} base ping URL (Enter keeps the saved URL): ")
    } else {
        format!("{label} base ping URL (https://hc-ping.com/...): ")
    };
    let mut candidate = config.clone();
    loop {
        let input = ask(&message)?;
        let url = input.trim();
        if url.is_empty() {
            if let Some(saved) = &saved {
                return Ok(saved.clone());
            }
            println!("Paste this check's ping URL and press Enter.");
            continue;
        }
        candidate.values.insert(key.into(), url.into());
        match candidate.ping_url(key, true) {
            Ok(url) => return Ok(url),
            Err(_) => println!("Enter the complete base URL from Healthchecks.io, starting with https://hc-ping.com/ and without a /start or /fail suffix."),
        }
    }
}

pub async fn background(tv: &str) -> Result<()> {
    let previous = Config::load(tv, false)?;
    let mut config = previous.clone();
    println!("Background features are optional. This computer must stay awake to run them.");
    println!("Home watching redirects the Home button and keeps the TV's app list current.");
    let watch = confirm("Enable Home watching? [y/N] ")?;
    println!("Session renewal reads the TV's Developer Mode token over SSH, asks LG to extend it, and verifies the remaining time daily. A cached token also works while the TV is off.");
    let renew = confirm("Enable automatic Developer Mode renewal? [y/N] ")?;
    if !watch && !renew {
        println!("No background jobs were changed. To stop existing jobs, run tvhome --tv {tv} uninstall.");
        return Ok(());
    }
    config.values.insert(
        "TV_WATCH_ENABLED".into(),
        if watch { "1" } else { "0" }.into(),
    );
    config.values.insert(
        "TV_RENEW_ENABLED".into(),
        if renew { "1" } else { "0" }.into(),
    );
    println!("Healthchecks.io can alert you when a selected job fails or this computer stops checking in.");
    let monitoring = confirm("Set up Healthchecks.io alerts? [y/N] ")?;
    let previous_alerts: Vec<_> = ["HC_WATCHER_URL", "HC_RENEWAL_URL"]
        .into_iter()
        .filter(|key| config.get(key).is_some())
        .collect();
    let mut monitoring_ready = false;
    if monitoring {
        println!("1. Open https://healthchecks.io/ and select your project.");
        println!("2. In Integrations, connect an alert destination (for example email), complete its verification and send a test notification.");
        println!("3. Create these checks with a Simple schedule:");
        if watch {
            println!("   {tv}-watch: period 5 minutes, grace 5 minutes.");
        }
        if renew {
            println!("   {tv}-renew: period 1 day, grace 4 hours.");
        }
        println!("4. Turn your notification integration ON for each check. Use a separate check for each job and TV.");
        println!("The period sets the expected ping interval; grace allows a delay before an alert. Ping URLs cannot configure or verify notification destinations.");
        monitoring_ready = confirm("Is the integration enabled for these checks, and did you receive its test notification? [y/N] ")?;
        if !monitoring_ready {
            println!(
                "Continuing without Healthchecks alerts. To add them later, rerun background."
            );
        }
    }
    for (enabled, key, label) in [
        (watch, "HC_WATCHER_URL", "Watcher"),
        (renew, "HC_RENEWAL_URL", "Renewal"),
    ] {
        if enabled && monitoring_ready {
            let url = prompt_ping_url(&config, key, label, prompt)?;
            config.values.insert(key.into(), url);
        } else {
            config.values.remove(key);
        }
    }
    crate::background::apply(&previous, &config).await?;
    if previous_alerts.iter().any(|key| config.get(key).is_none()) {
        println!("Pause the disabled checks in Healthchecks.io to avoid missed-ping alerts.");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ping_url_prompt_retries_empty_and_invalid_input() {
        let dir = tempfile::tempdir().unwrap();
        let config = crate::common::test_config(dir.path());
        let valid = "https://hc-ping.com/00000000-0000-0000-0000-000000000000";
        let inputs = [
            "".to_owned(),
            "not a URL".to_owned(),
            format!("{valid}/fail"),
            format!("  {valid}/  "),
        ];
        let mut inputs = inputs.into_iter();
        let mut attempts = 0;
        let result = prompt_ping_url(&config, "HC_WATCHER_URL", "Watcher", |message| {
            assert!(message.starts_with("Watcher base ping URL"));
            attempts += 1;
            Ok(inputs.next().expect("unexpected extra prompt"))
        })
        .unwrap();
        assert_eq!(result, valid);
        assert_eq!(attempts, 4);
        assert!(config.get("HC_WATCHER_URL").is_none());
    }
    #[test]
    fn ping_url_prompt_keeps_saved_value_on_enter() {
        let dir = tempfile::tempdir().unwrap();
        let mut config = crate::common::test_config(dir.path());
        let saved = "https://hc-ping.com/00000000-0000-0000-0000-000000000000";
        config.values.insert("HC_RENEWAL_URL".into(), saved.into());
        let result = prompt_ping_url(&config, "HC_RENEWAL_URL", "Renewal", |message| {
            assert!(message.contains("Enter keeps the saved URL"));
            Ok(String::new())
        })
        .unwrap();
        assert_eq!(result, saved);
    }
    #[test]
    fn ping_url_prompt_stops_when_input_closes() {
        let dir = tempfile::tempdir().unwrap();
        let config = crate::common::test_config(dir.path());
        let error = fault("Setup needs an interactive terminal.");
        assert_eq!(
            prompt_ping_url(&config, "HC_WATCHER_URL", "Watcher", |_| Err(error.clone())),
            Err(error)
        );
    }
    #[test]
    fn encrypted_developer_key_is_decrypted_without_saving_passphrase() {
        let rsa = openssl::rsa::Rsa::generate(2048).unwrap();
        let encrypted = rsa
            .private_key_to_pem_passphrase(openssl::symm::Cipher::aes_128_cbc(), b"secret")
            .unwrap();
        assert!(unlock(&encrypted, "wrong").is_err());
        let plain = unlock(&encrypted, "secret").unwrap();
        assert!(PKey::private_key_from_pem(&plain)
            .unwrap()
            .public_eq(&PKey::from_rsa(rsa).unwrap()));
        assert!(!String::from_utf8(plain).unwrap().contains("ENCRYPTED"));
    }
}
