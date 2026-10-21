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
pub struct Preferences {
    pub pinned_inputs: Vec<String>,
    #[serde(default)]
    pub tile_order: Vec<String>,
}
impl Preferences {
    pub fn contains(&self, id: &str) -> bool {
        self.pinned_inputs.iter().any(|p| p == id)
    }
    pub fn toggled(&self, id: &str) -> Self {
        let mut next = self.clone();
        if next.contains(id) {
            next.pinned_inputs.retain(|p| p != id);
            next.tile_order.retain(|p| p != id);
        } else {
            next.pinned_inputs.push(id.into());
        }
        next
    }
    pub fn with_order(&self, visible: &[String]) -> Self {
        let mut next = self.clone();
        // Keep slots for absent apps/inputs, while replacing visible slots in the new order.
        let visible_ids: std::collections::HashSet<_> = visible.iter().collect();
        let mut remaining = visible.iter();
        next.tile_order = self
            .tile_order
            .iter()
            .filter_map(|id| {
                if visible_ids.contains(id) {
                    remaining.next().cloned()
                } else {
                    Some(id.clone())
                }
            })
            .collect();
        next.tile_order.extend(remaining.cloned());
        // Bound history while always retaining every currently visible tile.
        let mut excess = next.tile_order.len().saturating_sub(1024);
        next.tile_order.retain(|id| {
            if excess > 0 && !visible_ids.contains(id) {
                excess -= 1;
                false
            } else {
                true
            }
        });
        next
    }
    fn validate(&self) -> Result<()> {
        ensure!(self.pinned_inputs.len() <= 65, "Too many pinned inputs");
        ensure!(self.tile_order.len() <= 1024, "Too many saved tiles");
        for ids in [&self.pinned_inputs, &self.tile_order] {
            let mut seen = std::collections::HashSet::new();
            for id in ids {
                ensure!(
                    !id.is_empty() && id.len() <= 512 && !id.chars().any(char::is_control),
                    "Invalid tile ID"
                );
                ensure!(seen.insert(id), "Duplicate tile ID");
            }
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
        file.take(1024 * 1024 + 1).read_to_end(&mut bytes)?;
        ensure!(bytes.len() <= 1024 * 1024, "Home preferences are too large");
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
                .context("Write Home preferences")?;
            file.sync_all().context("Flush Home preferences")?;
            fs::rename(&temp, path).context("Replace Home preferences")?;
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
        apply_order(&mut result, &self.tile_order);
        result
    }
}

pub fn tile_ids(catalog: &Catalog) -> Vec<String> {
    catalog.apps.iter().map(|app| app.id.clone()).collect()
}

pub fn apply_order(catalog: &mut Catalog, order: &[String]) {
    let ranks: std::collections::HashMap<_, _> = order
        .iter()
        .enumerate()
        .map(|(i, id)| (id.as_str(), i))
        .collect();
    // Stable sorting leaves newly discovered tiles in catalog order at the end.
    catalog
        .apps
        .sort_by_key(|app| ranks.get(app.id.as_str()).copied().unwrap_or(usize::MAX));
}

pub fn save_error(error: &anyhow::Error) -> String {
    let reason = error
        .chain()
        .find_map(|cause| cause.downcast_ref::<std::io::Error>());
    match reason.map(std::io::Error::kind) {
        Some(std::io::ErrorKind::PermissionDenied) => {
            "Could not save changes: storage permission denied.".into()
        }
        Some(std::io::ErrorKind::ReadOnlyFilesystem) => {
            "Could not save changes: storage is read-only.".into()
        }
        Some(std::io::ErrorKind::StorageFull) => {
            "Could not save changes: TV storage is full.".into()
        }
        _ => "Could not save changes. Try again.".into(),
    }
}

pub struct Writer {
    pub requests: mpsc::SyncSender<Preferences>,
    pub results: mpsc::Receiver<Result<Preferences>>,
}
pub fn writer(path: PathBuf) -> Writer {
    let (requests, queue) = mpsc::sync_channel::<Preferences>(1);
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
    fn ids(values: &[&str]) -> Vec<String> {
        values.iter().map(|id| (*id).into()).collect()
    }
    #[test]
    fn saved_order_mixes_apps_and_inputs_and_survives_catalog_changes() {
        let mut source = source();
        let mut second = source.apps[0].clone();
        second.id = "second".into();
        source.apps.push(second);
        let prefs = Preferences::default()
            .toggled("preview.hdmi1")
            .with_order(&ids(&["second", "preview.hdmi1", "preview.app"]));
        assert_eq!(
            tile_ids(&prefs.catalog(&source)),
            ids(&["second", "preview.hdmi1", "preview.app"])
        );
        source.apps.reverse();
        source.inputs[0].label = "Console".into();
        let mut added = source.apps[0].clone();
        added.id = "new".into();
        source.apps.insert(0, added);
        let updated = prefs.catalog(&source);
        assert_eq!(
            tile_ids(&updated),
            ids(&["second", "preview.hdmi1", "preview.app", "new"])
        );
        assert_eq!(updated.apps[1].title, "Console");
        let input = source.inputs.remove(0);
        source.apps.retain(|app| app.id != "second");
        assert_eq!(
            tile_ids(&prefs.catalog(&source)),
            ids(&["preview.app", "new"])
        );
        let changed = prefs.with_order(&ids(&["new", "preview.app"]));
        source.inputs.push(input);
        assert_eq!(
            tile_ids(&changed.catalog(&source)),
            ids(&["preview.hdmi1", "new", "preview.app"])
        );
        let unpinned = prefs.toggled("preview.hdmi1");
        assert!(!unpinned.tile_order.contains(&"preview.hdmi1".into()));
        assert_eq!(
            tile_ids(&unpinned.toggled("preview.hdmi1").catalog(&source)),
            ids(&["preview.app", "new", "preview.hdmi1"])
        );
    }
    #[test]
    fn preferences_are_per_tv_and_older_pin_files_remain_readable() {
        let old: Preferences =
            serde_json::from_str(r#"{"pinned_inputs":["preview.hdmi1"]}"#).unwrap();
        assert!(old.tile_order.is_empty());
        let root = std::env::temp_dir().join(format!("tvhome-order-{}", std::process::id()));
        let office = root.join("office/preferences.json");
        let guest = root.join("guest/preferences.json");
        let order = old.with_order(&ids(&["preview.hdmi1", "preview.app"]));
        order.save(&office).unwrap();
        old.save(&guest).unwrap();
        assert_eq!(Preferences::load(&office).unwrap(), order);
        assert_eq!(Preferences::load(&guest).unwrap(), old);
        let mut invalid = order.clone();
        invalid.tile_order.push("preview.app".into());
        assert!(invalid.save(&office).is_err());
        assert_eq!(Preferences::load(&office).unwrap(), order);
        // A real filesystem failure must not replace the saved preference file.
        fs::create_dir(office.with_extension("json.tmp")).unwrap();
        assert!(old.save(&office).is_err());
        assert_eq!(Preferences::load(&office).unwrap(), order);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn refreshed_catalog_keeps_pins_updates_labels_and_does_not_duplicate_apps() {
        let pins = Preferences::default().toggled("preview.hdmi1");
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
        let pins = Preferences::default()
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
        let pins = Preferences::default().toggled("preview.hdmi1");
        assert_eq!(Preferences::load(&path).unwrap(), Preferences::default());
        pins.save(&path).unwrap();
        assert_eq!(Preferences::load(&path).unwrap(), pins);
        let invalid = Preferences {
            pinned_inputs: vec!["duplicate".into(), "duplicate".into()],
            ..Default::default()
        };
        assert!(invalid.save(&path).is_err());
        assert_eq!(Preferences::load(&path).unwrap(), pins);
        fs::write(&path, b"{truncated").unwrap();
        assert!(Preferences::load(&path).is_err());
        fs::remove_dir_all(root).unwrap();
    }
}
