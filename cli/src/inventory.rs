use crate::{
    common::{fault, private_write, Config, FileLock, Result},
    package::{self, CatalogBundle, Package, APP_ID},
    ssh::{self, TvSsh},
    sync,
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};

const ASSETS: &str = "/media/developer/apps/usr/palm/applications/com.anant.tvhome.native/assets";
fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn encode(value: &Value) -> Result<Vec<u8>> {
    serde_json::to_vec_pretty(value).map_err(|_| fault("Cannot encode TV inventory."))
}
fn read(path: &Path) -> Result<Value> {
    serde_json::from_slice(&fs::read(path).map_err(|_| fault("Cannot read TV inventory."))?)
        .map_err(|_| fault("Invalid TV inventory."))
}
fn normalized(points: &[Value], devices: &[Value]) -> Value {
    let mut apps: Vec<Value> = points
        .iter()
        .filter(|p| p["id"].as_str().is_some_and(|s| !s.is_empty()))
        .map(|p| {
            json!({"id":p["id"],"title":p["title"],"hidden":p["hidden"] == true,
            "icon":p["icon"],"largeIcon":p["largeIcon"],"version":p["version"]})
        })
        .collect();
    apps.sort_by_key(|p| p["id"].as_str().unwrap().to_owned());
    apps.dedup_by(|a, b| a["id"] == b["id"]);
    let mut inputs = sync::inputs(devices);
    inputs.sort_by_key(|p| p["appId"].as_str().unwrap_or("").to_owned());
    json!({"apps":apps,"inputs":inputs})
}
pub fn record(config: &Config, points: &[Value], inputs: &[Value]) -> Result<Value> {
    let mut inventory = normalized(points, inputs);
    inventory["schema"] = 1.into();
    inventory["tv"] = config.tv.clone().into();
    inventory["host"] = config.host()?.into();
    inventory["checkedAt"] = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        .into();
    private_write(
        &config.state_dir().join("inventory.json"),
        &encode(&inventory)?,
    )?;
    Ok(inventory)
}
fn cached(config: &Config) -> Result<Value> {
    let value = read(&config.state_dir().join("inventory.json"))?;
    if value["schema"] != 1 || value["tv"] != config.tv || value["host"] != config.host()? {
        return Err(fault(
            "Inventory belongs to a different TV address. Run apps --refresh.",
        ));
    }
    Ok(value)
}
pub fn profile_names(home: &Path) -> Result<Vec<String>> {
    let root = home.join(".tvhome/tvs");
    let mut names = Vec::new();
    if root.exists() {
        for entry in fs::read_dir(root).map_err(|_| fault("Cannot list TV profiles."))? {
            let entry = entry.map_err(|_| fault("Cannot read TV profile."))?;
            if entry.path().join("environment.json").is_file() {
                names.push(entry.file_name().to_string_lossy().into_owned());
            }
        }
    }
    names.sort();
    Ok(names)
}
pub fn list_tvs(home: &Path, as_json: bool) -> Result<()> {
    let mut rows = Vec::new();
    for tv in profile_names(home)? {
        let config = Config::load_at(home.to_owned(), &tv, false, [])?;
        let inventory = cached(&config).unwrap_or(Value::Null);
        rows.push(json!({"name":tv,"host":config.host()?,
            "apps":inventory["apps"].as_array().map(Vec::len),"checkedAt":inventory["checkedAt"]}));
    }
    rows.sort_by_key(|r| r["name"].as_str().unwrap().to_owned());
    if as_json {
        println!("{}", json!(rows));
    } else {
        println!("{:<24} {:<24} CACHED APPS", "TV", "ADDRESS");
        for r in &rows {
            println!(
                "{:<24} {:<24} {}",
                r["name"].as_str().unwrap(),
                r["host"].as_str().unwrap(),
                r["apps"]
                    .as_u64()
                    .map(|n| n.to_string())
                    .unwrap_or_else(|| "not synced".into())
            );
        }
        if rows.is_empty() {
            println!("No saved TVs. Run tvhome --tv living-room setup.");
        }
    }
    Ok(())
}
pub async fn list_apps(config: &Config, refresh: bool, as_json: bool) -> Result<()> {
    if refresh {
        let template = Package::read(package::bundled())?;
        update(config, &template).await?;
    }
    let value = cached(config)?;
    if as_json {
        println!("{value}");
    } else {
        println!(
            "{} ({}) — cached app inventory; use --refresh to check now.",
            config.tv,
            config.host()?
        );
        for p in value["apps"]
            .as_array()
            .ok_or(fault("Invalid app inventory."))?
        {
            println!(
                "{:<48} {}",
                p["id"].as_str().unwrap_or(""),
                p["title"].as_str().unwrap_or("")
            );
        }
    }
    Ok(())
}

trait Remote {
    fn exec(&self, command: &str) -> Result<Vec<u8>>;
    fn upload(&self, path: &str, bytes: &[u8]) -> Result<()>;
}
impl Remote for TvSsh {
    fn exec(&self, command: &str) -> Result<Vec<u8>> {
        self.command(command)
    }
    fn upload(&self, path: &str, bytes: &[u8]) -> Result<()> {
        TvSsh::upload(self, path, bytes)
    }
}
fn checksum(remote: &impl Remote, path: &str) -> Result<String> {
    let output = remote.exec(&format!("sha256sum {}", ssh::quote(path)))?;
    let value = std::str::from_utf8(&output)
        .map_err(|_| fault("Invalid TV checksum."))?
        .split_whitespace()
        .next()
        .unwrap_or("");
    if value.len() != 64 || !value.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(fault("TV did not return a valid catalog checksum."));
    }
    Ok(value.to_ascii_lowercase())
}
fn publish(remote: &impl Remote, bundle: &CatalogBundle) -> Result<()> {
    for (name, bytes) in &bundle.artwork {
        // All names are derived from bytes, never from a TV-supplied app title or ID.
        if *name != format!("tvhome-{}.png", digest(bytes)) {
            return Err(fault("Invalid artwork content name."));
        }
        let path = format!("{ASSETS}/{name}");
        if checksum(remote, &path).ok().as_deref() == Some(&digest(bytes)) {
            continue;
        }
        let stage = format!("{path}.part");
        remote.upload(&stage, bytes)?;
        if checksum(remote, &stage)? != digest(bytes) {
            return Err(fault("Uploaded artwork checksum does not match."));
        }
        remote.exec(&format!(
            "chmod 644 {} && mv -f {} {}",
            ssh::quote(&stage),
            ssh::quote(&stage),
            ssh::quote(&path)
        ))?;
    }
    let stage = format!("{ASSETS}/catalog.json.part");
    remote.upload(&stage, &bundle.json)?;
    if checksum(remote, &stage)? != digest(&bundle.json) {
        return Err(fault("Uploaded catalog checksum does not match."));
    }
    // The only commit point: readers see the old complete catalog or the new complete catalog.
    remote.exec(&format!(
        "chmod 644 {} && mv -f {} {}",
        ssh::quote(&stage),
        ssh::quote(&stage),
        ssh::quote(&format!("{ASSETS}/catalog.json"))
    ))?;
    Ok(())
}
fn remote_catalog(remote: &impl Remote) -> Result<String> {
    let bytes = remote.exec(&format!(
        "cat {}",
        ssh::quote(&format!("{ASSETS}/../appinfo.json"))
    ))?;
    let app: Value = serde_json::from_slice(&bytes)
        .map_err(|_| fault("Cannot read installed Home metadata."))?;
    let version: Vec<u32> = app["version"]
        .as_str()
        .unwrap_or("")
        .split('.')
        .filter_map(|s| s.parse().ok())
        .collect();
    if app["id"] != APP_ID || version.len() != 3 || version.as_slice() < [0, 1, 0].as_slice() {
        return Err(fault(
            "Install Home with setup to enable automatic app-list updates.",
        ));
    }
    checksum(remote, &format!("{ASSETS}/catalog.json"))
}
fn needs_publish(saved: &Value, host: &str, source: &str, remote: &str) -> bool {
    saved["host"] != host || saved["source"] != source || saved["catalog"] != remote
}
pub async fn update(config: &Config, template: &Package) -> Result<bool> {
    let _lock = FileLock::acquire(&config.state_dir().join("catalog.lock"))?;
    let (points, inputs) = sync::capture(config, false).await?;
    record(config, &points, &inputs)?;
    let source = digest(&encode(&json!({"inventory":normalized(&points,&inputs),
        "tvhome":env!("CARGO_PKG_VERSION"),"policy":template.policy()?}))?);
    let worker = config.clone();
    let current =
        tokio::task::spawn_blocking(move || remote_catalog(&TvSsh::connect(&worker, false)?))
            .await
            .map_err(|_| fault("Cannot check the TV catalog."))??;
    let publication = config.state_dir().join("publication.json");
    let saved = if publication.exists() {
        read(&publication)?
    } else {
        Value::Null
    };
    if !needs_publish(&saved, config.host()?, &source, &current) {
        return Ok(false);
    }
    let bundle = template.catalog(config, &points, &inputs).await?;
    let published = digest(&bundle.json);
    if current != published {
        let worker = config.clone();
        tokio::task::spawn_blocking(move || publish(&TvSsh::connect(&worker, false)?, &bundle))
            .await
            .map_err(|_| fault("Catalog upload task failed."))??;
    }
    private_write(
        &publication,
        &encode(&json!({"host":config.host()?,"source":source,"catalog":published}))?,
    )?;
    crate::log(&format!("Updated the app catalog for {}.", config.tv));
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    #[test]
    fn normalization_ignores_order_but_detects_install_remove_and_rename() {
        let a = json!({"id":"a","title":"A"});
        let b = json!({"id":"b","title":"B"});
        assert_eq!(
            normalized(&[a.clone(), b.clone()], &[]),
            normalized(&[b.clone(), a.clone()], &[])
        );
        assert_ne!(
            normalized(std::slice::from_ref(&a), &[]),
            normalized(&[a.clone(), b], &[])
        );
        assert_ne!(
            normalized(&[a], &[]),
            normalized(&[json!({"id":"a","title":"New"})], &[])
        );
    }
    #[test]
    fn inventories_are_isolated_and_reject_changed_addresses() {
        let dir = tempfile::tempdir().unwrap();
        let a = crate::common::test_config(dir.path());
        let mut b = a.clone();
        b.tv = "bedroom".into();
        record(&a, &[json!({"id":"netflix"})], &[]).unwrap();
        record(&b, &[json!({"id":"youtube"})], &[]).unwrap();
        assert_eq!(cached(&a).unwrap()["apps"][0]["id"], "netflix");
        assert_eq!(cached(&b).unwrap()["apps"][0]["id"], "youtube");
        b.values.insert("TV_HOST".into(), "192.0.2.90".into());
        assert!(cached(&b).is_err());
    }
    #[test]
    fn unchanged_catalogs_skip_upload_but_reinstalls_and_failures_retry() {
        let saved = json!({"host":"tv","source":"source1","catalog":"published1"});
        assert!(!needs_publish(&saved, "tv", "source1", "published1"));
        assert!(needs_publish(&saved, "tv", "source2", "published1"));
        assert!(needs_publish(&saved, "tv", "source1", "reinstalled"));
        assert!(needs_publish(&Value::Null, "tv", "source1", "published1"));
    }
    struct Fake {
        files: RefCell<std::collections::BTreeMap<String, Vec<u8>>>,
        commits: RefCell<Vec<String>>,
        fail: bool,
    }
    impl Remote for Fake {
        fn upload(&self, path: &str, bytes: &[u8]) -> Result<()> {
            if self.fail && path.ends_with(".png.part") {
                return Err(fault("interrupted"));
            }
            self.files.borrow_mut().insert(path.into(), bytes.into());
            Ok(())
        }
        fn exec(&self, command: &str) -> Result<Vec<u8>> {
            if command.starts_with("cat ") {
                let path = command.trim_start_matches("cat ").trim_matches('\'');
                return self
                    .files
                    .borrow()
                    .get(path)
                    .cloned()
                    .ok_or(fault("missing"));
            }
            if command.starts_with("sha256sum ") {
                let path = command.trim_start_matches("sha256sum ").trim_matches('\'');
                return self
                    .files
                    .borrow()
                    .get(path)
                    .map(|b| digest(b).into_bytes())
                    .ok_or(fault("missing"));
            }
            if command.starts_with("chmod ") {
                self.commits.borrow_mut().push(command.into());
                return Ok(vec![]);
            }
            Err(fault("unexpected command"))
        }
    }
    #[test]
    fn current_home_release_supports_catalog_updates() {
        let remote = Fake {
            files: RefCell::new(std::collections::BTreeMap::from([
                (
                    format!("{ASSETS}/../appinfo.json"),
                    include_bytes!("../../tv/packaging/appinfo.json").to_vec(),
                ),
                (format!("{ASSETS}/catalog.json"), b"catalog".to_vec()),
            ])),
            commits: Default::default(),
            fail: false,
        };
        assert_eq!(remote_catalog(&remote).unwrap(), digest(b"catalog"));
        assert!(remote.commits.borrow().is_empty());
    }
    #[test]
    fn interrupted_icon_upload_never_commits_catalog_and_success_commits_it_last() {
        let data = b"icon".to_vec();
        let bundle = CatalogBundle {
            json: b"catalog".to_vec(),
            artwork: std::collections::BTreeMap::from([(
                format!("tvhome-{}.png", digest(&data)),
                data,
            )]),
        };
        let mut remote = Fake {
            files: Default::default(),
            commits: Default::default(),
            fail: true,
        };
        assert!(publish(&remote, &bundle).is_err());
        assert!(remote.commits.borrow().is_empty());
        remote.fail = false;
        publish(&remote, &bundle).unwrap();
        let commits = remote.commits.borrow();
        assert_eq!(commits.len(), 2);
        assert!(commits[0].contains(".png.part"));
        assert!(commits[1].ends_with("/catalog.json'"));
        assert!(!commits
            .iter()
            .any(|c| c.contains("launch") || c.contains("install")));
    }
}
