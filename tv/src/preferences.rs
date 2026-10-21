use crate::model::{App, Catalog};
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::mpsc,
};

#[derive(Clone, Default, Deserialize, Serialize, Debug, PartialEq)]
pub struct Pins {
    pub pinned_inputs: Vec<String>,
}
impl Pins {
    pub fn contains(&self, id: &str) -> bool {
        self.pinned_inputs.iter().any(|p| p == id)
    }
    pub fn toggled(&self, id: &str) -> Self {
        let mut next = self.clone();
        if next.contains(id) {
            next.pinned_inputs.retain(|p| p != id);
        } else {
            next.pinned_inputs.push(id.into());
        }
        next
    }
    fn validate(&self) -> Result<()> {
        ensure!(self.pinned_inputs.len() <= 65, "Too many pinned inputs");
        let mut seen = std::collections::HashSet::new();
        for id in &self.pinned_inputs {
            ensure!(
                !id.is_empty() && id.len() <= 512 && !id.chars().any(char::is_control),
                "Invalid pinned input ID"
            );
            ensure!(seen.insert(id), "Duplicate pinned input ID");
        }
        Ok(())
    }
    pub fn load(path: &Path) -> Result<Self> {
        let file = match fs::File::open(path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::default())
            }
            Err(error) => return Err(error.into()),
        };
        let mut bytes = Vec::new();
        file.take(65537).read_to_end(&mut bytes)?;
        ensure!(bytes.len() <= 65536, "Pinned input settings are too large");
        let pins: Self = serde_json::from_slice(&bytes)?;
        pins.validate()?;
        Ok(pins)
    }
    fn save(&self, path: &Path) -> Result<()> {
        self.validate()?;
        fs::create_dir_all(path.parent().context("Missing settings directory")?)
            .with_context(|| format!("Create settings directory for {}", path.display()))?;
        let temp = path.with_extension("json.tmp");
        let result = (|| -> Result<()> {
            let mut file =
                fs::File::create(&temp).with_context(|| format!("Create {}", temp.display()))?;
            file.write_all(&serde_json::to_vec_pretty(self)?)
                .context("Write pinned inputs")?;
            file.sync_all().context("Flush pinned inputs")?;
            fs::rename(&temp, path).context("Replace pinned inputs")?;
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(temp);
        }
        result
    }
    /// Overlay local choices without modifying the catalog published by tvhome.
    pub fn catalog(&self, source: &Catalog) -> Catalog {
        let mut result = source.clone();
        let inputs = source.input_items();
        for id in &self.pinned_inputs {
            if result.apps.iter().any(|app| &app.id == id) {
                continue;
            }
            if let Some(input) = inputs.iter().find(|input| &input.app_id == id) {
                result.apps.push(App {
                    id: id.clone(),
                    title: input.label.clone(),
                    artwork: String::new(),
                    contain: false,
                    color: [0x11, 0x11, 0x11],
                    background: None,
                    input: true,
                });
            }
        }
        result
    }
}

pub fn save_error(error: &anyhow::Error) -> String {
    let reason = error
        .chain()
        .find_map(|cause| cause.downcast_ref::<std::io::Error>());
    match reason.map(std::io::Error::kind) {
        Some(std::io::ErrorKind::PermissionDenied) => {
            "Could not save pins: storage permission denied.".into()
        }
        Some(std::io::ErrorKind::ReadOnlyFilesystem) => {
            "Could not save pins: storage is read-only.".into()
        }
        Some(std::io::ErrorKind::StorageFull) => "Could not save pins: TV storage is full.".into(),
        _ => "Could not save pinned inputs. Try again.".into(),
    }
}

pub struct Writer {
    pub requests: mpsc::SyncSender<Pins>,
    pub results: mpsc::Receiver<Result<Pins>>,
}
pub fn writer(path: PathBuf) -> Writer {
    let (requests, queue) = mpsc::sync_channel::<Pins>(1);
    let (tx, results) = mpsc::channel();
    std::thread::spawn(move || {
        for pins in queue {
            let result = pins.save(&path).map(|()| pins);
            if tx.send(result).is_err() {
                break;
            }
        }
    });
    Writer { requests, results }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn source() -> Catalog {
        serde_json::from_value(serde_json::json!({"preview":true,"apps":[
            {"id":"preview.app","title":"App","artwork":"","contain":false,"color":[0,0,0]}
        ],"inputs":[{"appId":"preview.hdmi1","label":"HDMI 1"}]}))
        .unwrap()
    }
    #[test]
    fn refreshed_catalog_keeps_pins_updates_labels_and_does_not_duplicate_apps() {
        let pins = Pins::default().toggled("preview.hdmi1");
        let mut source = source();
        assert_eq!(pins.catalog(&source).apps[1].title, "HDMI 1");
        source.inputs[0].label = "Console".into();
        assert_eq!(pins.catalog(&source).apps[1].title, "Console");
        source.apps = pins.catalog(&source).apps;
        assert_eq!(pins.catalog(&source).apps.len(), 2);
        source.apps.truncate(1);
        let input = source.inputs.remove(0);
        assert_eq!(pins.catalog(&source).apps.len(), 1);
        source.inputs.push(input);
        assert!(pins.catalog(&source).apps[1].input);
        assert_eq!(pins.toggled("preview.hdmi1").catalog(&source).apps.len(), 1);
    }
    #[test]
    fn live_tv_can_be_pinned_and_order_follows_pin_order() {
        let pins = Pins::default()
            .toggled("com.webos.app.livetv")
            .toggled("preview.hdmi1");
        let result = pins.catalog(&source());
        assert_eq!(result.apps[1].id, "com.webos.app.livetv");
        assert_eq!(result.apps[2].id, "preview.hdmi1");
    }
    #[test]
    fn persistence_roundtrip_and_rejected_write_preserves_saved_choices() {
        let root = std::env::temp_dir().join(format!(
            "tvhome-pins-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let path = root.join("preferences.json");
        let pins = Pins::default().toggled("preview.hdmi1");
        assert_eq!(Pins::load(&path).unwrap(), Pins::default());
        pins.save(&path).unwrap();
        assert_eq!(Pins::load(&path).unwrap(), pins);
        let invalid = Pins {
            pinned_inputs: vec!["duplicate".into(), "duplicate".into()],
        };
        assert!(invalid.save(&path).is_err());
        assert_eq!(Pins::load(&path).unwrap(), pins);
        fs::write(&path, b"{truncated").unwrap();
        assert!(Pins::load(&path).is_err());
        fs::remove_dir_all(root).unwrap();
    }
}
