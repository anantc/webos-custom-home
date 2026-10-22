use crate::{
    common::{fault, Result},
    sync,
};
use flate2::{read::GzDecoder, write::GzEncoder, Compression};
use regex::Regex;
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, HashSet},
    io::{Cursor, Read},
    path::Component,
};

pub const APP_ID: &str = "com.anant.tvhome.native";
const ROOT: &str = "usr/palm/applications/com.anant.tvhome.native/";
const LIMIT: u64 = 128 * 1024 * 1024;
type Files = BTreeMap<String, (u32, Vec<u8>)>;

pub fn bundled() -> &'static [u8] {
    include_bytes!(concat!(env!("OUT_DIR"), "/home.ipk"))
}
fn bounded(reader: impl Read, limit: u64) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    reader
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| fault("Cannot read Home package."))?;
    if bytes.len() as u64 > limit {
        return Err(fault("Home package exceeds its size limit."));
    }
    Ok(bytes)
}
fn unpack_tar(bytes: &[u8]) -> Result<Files> {
    let expanded = bounded(GzDecoder::new(bytes), LIMIT)?;
    let mut archive = tar::Archive::new(Cursor::new(expanded));
    let mut files = Files::new();
    for item in archive
        .entries()
        .map_err(|_| fault("Invalid package archive."))?
    {
        let mut entry = item.map_err(|_| fault("Invalid package entry."))?;
        let raw = entry
            .path()
            .map_err(|_| fault("Invalid package path."))?
            .into_owned();
        if raw.components().any(|c| {
            matches!(
                c,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        }) {
            return Err(fault("Unsafe package path."));
        }
        let mut name = raw
            .components()
            .filter_map(|c| match c {
                Component::Normal(s) => s.to_str(),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("/");
        if name.is_empty() {
            continue;
        }
        let directory = entry.header().entry_type().is_dir();
        if directory {
            name.push('/');
        }
        if !directory && !entry.header().entry_type().is_file() {
            return Err(fault(
                "Package contains unsupported links or special files.",
            ));
        }
        let mode = entry
            .header()
            .mode()
            .map_err(|_| fault("Invalid package permissions."))?;
        let content = bounded(&mut entry, LIMIT)?;
        if directory && !content.is_empty() {
            return Err(fault("Package directory contains unexpected data."));
        }
        if files.insert(name, (mode, content)).is_some() || files.len() > 4096 {
            return Err(fault("Duplicate or excessive package entries."));
        }
    }
    Ok(files)
}
fn pack_tar(files: &Files) -> Result<Vec<u8>> {
    let mut builder = tar::Builder::new(GzEncoder::new(Vec::new(), Compression::fast()));
    for (name, (mode, bytes)) in files {
        let mut header = tar::Header::new_gnu();
        header.set_size(bytes.len() as u64);
        header.set_entry_type(if name.ends_with('/') {
            tar::EntryType::Directory
        } else {
            tar::EntryType::Regular
        });
        header.set_mode(*mode);
        header.set_uid(0);
        header.set_gid(0);
        header.set_mtime(0);
        header.set_cksum();
        builder
            .append_data(&mut header, name, bytes.as_slice())
            .map_err(|_| fault("Cannot assemble Home package."))?;
    }
    builder
        .into_inner()
        .map_err(|_| fault("Cannot finish Home archive."))?
        .finish()
        .map_err(|_| fault("Cannot compress Home package."))
}
fn member<'a>(files: &'a Files, suffix: &str) -> Result<&'a [u8]> {
    files
        .get(&format!("{ROOT}{suffix}"))
        .map(|(_, bytes)| bytes.as_slice())
        .ok_or(fault("Home package is missing a required file."))
}
pub struct CatalogBundle {
    pub json: Vec<u8>,
    pub artwork: BTreeMap<String, Vec<u8>>,
}
pub struct Package {
    data: Files,
    control: Files,
}
impl Package {
    pub fn version(&self) -> Result<String> {
        let info: Value = serde_json::from_slice(member(&self.data, "appinfo.json")?)
            .map_err(|_| fault("Invalid Home app metadata."))?;
        info["version"]
            .as_str()
            .map(str::to_owned)
            .ok_or(fault("Home package has no version."))
    }

    pub fn preserve_state(&mut self, files: &BTreeMap<String, Vec<u8>>) -> Result<()> {
        for (name, bytes) in files {
            if !["preferences.json", "speedtest-result.json"].contains(&name.as_str()) {
                return Err(fault("Unsupported Home state file."));
            }
            self.data
                .insert(format!("{ROOT}state/{name}"), (0o644, bytes.clone()));
        }
        Ok(())
    }
    pub fn read(bytes: &[u8]) -> Result<Self> {
        if bytes.is_empty() {
            return Err(fault("This development build has no Home package. Build with bundled-home or supply --package."));
        }
        if bytes.len() as u64 > LIMIT {
            return Err(fault("Home package exceeds its size limit."));
        }
        let mut archive = ar::Archive::new(Cursor::new(bytes));
        let mut members = BTreeMap::new();
        while let Some(entry) = archive.next_entry() {
            let mut entry = entry.map_err(|_| fault("Invalid IPK archive."))?;
            let name = std::str::from_utf8(entry.header().identifier())
                .map_err(|_| fault("Invalid IPK member."))?
                .trim_end_matches('/')
                .to_owned();
            if !["debian-binary", "control.tar.gz", "data.tar.gz"].contains(&name.as_str())
                || members.contains_key(&name)
            {
                return Err(fault("Unexpected or duplicate IPK member."));
            }
            members.insert(name, bounded(&mut entry, LIMIT)?);
        }
        if members.get("debian-binary").map(Vec::as_slice) != Some(b"2.0\n") {
            return Err(fault("Invalid IPK version."));
        }
        let data = unpack_tar(
            members
                .get("data.tar.gz")
                .ok_or(fault("Missing package data."))?,
        )?;
        let control = unpack_tar(
            members
                .get("control.tar.gz")
                .ok_or(fault("Missing package metadata."))?,
        )?;
        let info: Value = serde_json::from_slice(member(&data, "appinfo.json")?)
            .map_err(|_| fault("Invalid Home app metadata."))?;
        if info["id"] != APP_ID || info["type"] != "native" {
            return Err(fault("Package is not the native Home app."));
        }
        let metadata = std::str::from_utf8(
            &control
                .get("control")
                .ok_or(fault("Missing package control."))?
                .1,
        )
        .map_err(|_| fault("Invalid package control."))?;
        if !metadata.lines().any(|l| l == format!("Package: {APP_ID}"))
            || !metadata.lines().any(|l| l == "Architecture: arm")
        {
            return Err(fault(
                "Package identity or architecture does not match Home.",
            ));
        }
        let binary = member(&data, "bin/tvhome-native")?;
        if binary.len() < 40
            || &binary[..6] != b"\x7fELF\x01\x01"
            || u16::from_le_bytes([binary[18], binary[19]]) != 40
        {
            return Err(fault("Home package has no ARM executable."));
        }
        if data[&format!("{ROOT}bin/tvhome-native")].0 & 0o111 == 0 {
            return Err(fault("Home executable is not executable."));
        }
        Ok(Self { data, control })
    }
    pub fn policy(&self) -> Result<Value> {
        let policy: Value = serde_json::from_slice(member(&self.data, "assets/setup.json")?)
            .map_err(|_| fault("Invalid Home setup metadata."))?;
        if policy["schema"] != 1 || !policy["brands"].is_array() {
            return Err(fault("Unsupported Home setup metadata."));
        }
        Ok(policy)
    }
    pub async fn catalog(
        &self,
        config: &crate::common::Config,
        points: &[Value],
        inputs: &[Value],
    ) -> Result<CatalogBundle> {
        use sha2::{Digest, Sha256};
        let mut apps = ordered(points, &self.policy()?)?;
        let mut artwork = BTreeMap::new();
        for app in &mut apps {
            let name = app["artwork"].as_str().unwrap_or("");
            let bytes = if name.is_empty() {
                match points.iter().find(|p| p["id"] == app["id"]) {
                    Some(point) => sync::icon(config, point).await.unwrap_or_default(),
                    None => Vec::new(),
                }
            } else {
                member(&self.data, &format!("assets/{name}"))?.to_vec()
            };
            if !bytes.is_empty() {
                // Content names let an old reader finish while the next catalog is published.
                let name = format!("tvhome-{:x}.png", Sha256::digest(&bytes));
                artwork.insert(name.clone(), bytes);
                app["artwork"] = name.into();
            }
        }
        if apps.is_empty() {
            return Err(fault("The TV returned no visible apps for Home."));
        }
        Ok(CatalogBundle {
            json: serde_json::to_vec(
                &json!({"preview":false,"apps":apps,"inputs":sync::inputs(inputs)}),
            )
            .map_err(|_| fault("Cannot encode Home catalog."))?,
            artwork,
        })
    }
    pub fn personalize(mut self, catalog: &CatalogBundle) -> Result<Vec<u8>> {
        self.data.insert(
            format!("{ROOT}assets/catalog.json"),
            (0o644, catalog.json.clone()),
        );
        for (name, bytes) in &catalog.artwork {
            self.data
                .insert(format!("{ROOT}assets/{name}"), (0o644, bytes.clone()));
        }
        self.data.remove(&format!("{ROOT}assets/setup.json"));
        self.finish()
    }
    #[cfg(all(test, feature = "bundled-home"))]
    pub async fn prepare(
        self,
        config: &crate::common::Config,
        points: &[Value],
        inputs: &[Value],
    ) -> Result<Vec<u8>> {
        let catalog = self.catalog(config, points, inputs).await?;
        self.personalize(&catalog)
    }
    pub(crate) fn finish(mut self) -> Result<Vec<u8>> {
        let size: usize = self.data.values().map(|(_, bytes)| bytes.len()).sum();
        let control = &mut self
            .control
            .get_mut("control")
            .ok_or(fault("Missing package control."))?
            .1;
        let text = std::str::from_utf8(control).map_err(|_| fault("Invalid package control."))?;
        *control = text
            .lines()
            .map(|line| {
                if line.starts_with("Installed-Size:") {
                    format!("Installed-Size: {size}")
                } else {
                    line.to_owned()
                }
            })
            .collect::<Vec<_>>()
            .join("\n")
            .into_bytes();
        control.push(b'\n');
        self.control.remove("md5sums");
        let mut archive = ar::Builder::new(Vec::new());
        for (name, bytes) in [
            ("debian-binary", b"2.0\n".to_vec()),
            ("control.tar.gz", pack_tar(&self.control)?),
            ("data.tar.gz", pack_tar(&self.data)?),
        ] {
            let mut header = ar::Header::new(name.as_bytes().to_vec(), bytes.len() as u64);
            header.set_mode(0o100644);
            archive
                .append(&header, bytes.as_slice())
                .map_err(|_| fault("Cannot assemble IPK."))?;
        }
        archive
            .into_inner()
            .map_err(|_| fault("Cannot finish IPK."))
    }
}
pub fn ordered(points: &[Value], policy: &Value) -> Result<Vec<Value>> {
    let brands = policy["brands"]
        .as_array()
        .ok_or(fault("Missing brand definitions."))?;
    let patterns = brands
        .iter()
        .map(|b| {
            Regex::new(b["pattern"].as_str().unwrap_or("(?!)"))
                .map_err(|_| fault("Invalid brand matching rule."))
        })
        .collect::<Result<Vec<_>>>()?;
    let excluded = policy["config"]["hiddenApps"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let mut seen = HashSet::new();
    let mut pool = Vec::new();
    for point in points {
        let Some(record) = sync::app_record(point, "") else {
            continue;
        };
        let id = record["id"].as_str().unwrap();
        if id == APP_ID
            || id == "com.palmdts.devmode"
            || id.starts_with("preview.")
            || excluded.iter().any(|v| v == id)
            || !seen.insert(id.to_owned())
        {
            continue;
        }
        let title = record["title"].as_str().unwrap();
        let brand = patterns
            .iter()
            .position(|re| re.is_match(title) || re.is_match(id))
            .map(|i| &brands[i]);
        let app = if let Some(b) = brand {
            json!({"id":id,"title":b["name"],"artwork":b["artwork"],"contain":b["contain"],"color":b["color"],"background":b["background"]})
        } else {
            json!({"id":id,"title":title,"artwork":"","contain":true,"color":[95,113,158]})
        };
        pool.push((
            brand.and_then(|b| b["key"].as_str()).map(str::to_owned),
            app,
        ));
    }
    let mut result = Vec::new();
    for (field, by_brand) in [("dockApps", false), ("preferredApps", true)] {
        if let Some(keys) = policy["config"][field].as_array() {
            for key in keys.iter().filter_map(Value::as_str) {
                if let Some(index) = pool.iter().position(|(brand, app)| {
                    if by_brand {
                        brand.as_deref() == Some(key)
                    } else {
                        app["id"] == key
                    }
                }) {
                    result.push(pool.remove(index).1);
                }
            }
        }
    }
    pool.sort_by_key(|(_, app)| app["title"].as_str().unwrap_or("").to_lowercase());
    result.extend(pool.into_iter().map(|(_, app)| app));
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(feature = "bundled-home")]
    #[tokio::test]
    async fn update_package_preserves_each_tvs_state_and_writable_directory() {
        let root = tempfile::tempdir().unwrap();
        for tv in ["office", "guestroom"] {
            let mut config = crate::common::test_config(root.path());
            config.tv = tv.into();
            let mut package = Package::read(bundled()).unwrap();
            let version = package.version().unwrap();
            let saved = BTreeMap::from([
                (
                    "preferences.json".into(),
                    json!({"pinnedInputs":["hdmi1"],"order":["hdmi1","netflix"],"city":tv})
                        .to_string()
                        .into_bytes(),
                ),
                ("speedtest-result.json".into(), b"{\"mbps\":210}".to_vec()),
            ]);
            package.preserve_state(&saved).unwrap();
            assert!(package
                .preserve_state(&BTreeMap::from([("../appinfo.json".into(), vec![])]))
                .is_err());
            let bytes = package
                .prepare(&config, &[json!({"id":"netflix","title":"Netflix"})], &[])
                .await
                .unwrap();
            let installed = Package::read(&bytes).unwrap();
            assert_eq!(installed.version().unwrap(), version);
            assert_eq!(installed.data[&format!("{ROOT}state/")].0 & 0o777, 0o777);
            for (name, bytes) in saved {
                assert_eq!(
                    member(&installed.data, &format!("state/{name}")).unwrap(),
                    bytes
                );
                assert_eq!(installed.data[&format!("{ROOT}state/{name}")].0, 0o644);
            }
        }
    }
    #[test]
    fn catalog_uses_real_ids_and_filters_duplicates_and_developer_mode() {
        let policy = json!({"brands":[{"key":"netflix","name":"Netflix","pattern":"(?i)netflix","artwork":"brand-netflix.jpg","contain":false,"color":[1,2,3]}],"config":{"preferredApps":["netflix"],"hiddenApps":["hidden"]}});
        let apps = ordered(
            &[
                json!({"id":"z","title":"Zed"}),
                json!({"id":"real.netflix","title":"Netflix"}),
                json!({"id":"real.netflix","title":"Duplicate"}),
                json!({"id":"com.palmdts.devmode"}),
                json!({"id":APP_ID}),
                json!({"id":"hidden"}),
                json!({"id":"secret","hidden":true}),
                json!({"id":"a","title":"Alpha"}),
            ],
            &policy,
        )
        .unwrap();
        assert_eq!(
            apps.iter()
                .map(|v| v["id"].as_str().unwrap())
                .collect::<Vec<_>>(),
            vec!["real.netflix", "a", "z"]
        );
        assert_eq!(apps[0]["artwork"], "brand-netflix.jpg");
    }
    #[test]
    fn archives_reject_excessive_data_and_arbitrary_input() {
        assert!(bounded(Cursor::new(b"long"), 2).is_err());
        assert!(Package::read(b"not an ipk").is_err());
        assert!(Package::read(&[]).is_err());
    }
    #[test]
    fn tar_round_trip_preserves_file_and_directory_permissions() {
        // Without directory entries, extraction creates root-owned 0755 parents,
        // preventing the TV's unprivileged app from saving settings.
        let files = BTreeMap::from([
            ("app/".into(), (0o755, Vec::new())),
            ("app/bin/".into(), (0o755, Vec::new())),
            ("app/bin/home".into(), (0o755, b"binary".to_vec())),
            ("app/state/".into(), (0o777, Vec::new())),
        ]);
        let archive = pack_tar(&files).unwrap();
        assert_eq!(unpack_tar(&archive).unwrap(), files);
        let dir = tempfile::tempdir().unwrap();
        tar::Archive::new(GzDecoder::new(archive.as_slice()))
            .unpack(dir.path())
            .unwrap();
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(dir.path().join("app/state"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o777
        );
        assert_eq!(
            std::fs::metadata(dir.path().join("app/bin/home"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o755
        );
    }
    #[cfg(feature = "bundled-home")]
    #[tokio::test]
    async fn bundled_template_personalizes_without_preview_or_tv_identity() {
        let dir = tempfile::tempdir().unwrap();
        let config = crate::common::test_config(dir.path());
        let bytes = Package::read(bundled())
            .unwrap()
            .prepare(
                &config,
                &[json!({"id":"netflix","title":"Netflix"})],
                &[json!({"appId":"com.webos.app.hdmi1","label":"Console"})],
            )
            .await
            .unwrap();
        let result = Package::read(&bytes).unwrap();
        let state = result
            .data
            .get(&format!("{ROOT}state/"))
            .expect("Personalization must preserve the writable state directory");
        assert_eq!(state.0 & 0o777, 0o777);
        assert!(state.1.is_empty());
        let catalog: Value =
            serde_json::from_slice(member(&result.data, "assets/catalog.json").unwrap()).unwrap();
        assert_eq!(catalog["preview"], false);
        assert_eq!(catalog["apps"][0]["id"], "netflix");
        assert_eq!(catalog["inputs"][0]["label"], "Console");
        assert!(result.policy().is_err());
    }
}
