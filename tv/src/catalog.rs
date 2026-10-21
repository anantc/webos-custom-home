use crate::{model::Catalog, render};
use anyhow::{Context, Result};
use skia_safe::Image;
use std::{io::Read, path::PathBuf, sync::mpsc};

pub struct Update {
    pub catalog: Catalog,
    pub icons: Vec<Option<Image>>,
}
/// Artwork follows stable app IDs, including when an input is inserted among apps.
pub fn icons_for<T: Clone>(
    source: &Catalog,
    icons: &[Option<T>],
    displayed: &Catalog,
) -> Vec<Option<T>> {
    let by_id: std::collections::HashMap<_, _> = source
        .apps
        .iter()
        .enumerate()
        .map(|(i, app)| (app.id.as_str(), i))
        .collect();
    displayed
        .apps
        .iter()
        .map(|app| {
            if app.input {
                return None;
            }
            by_id
                .get(app.id.as_str())
                .and_then(|i| icons.get(*i))
                .cloned()
                .flatten()
        })
        .collect()
}
pub struct Worker {
    pub requests: mpsc::SyncSender<()>,
    pub results: mpsc::Receiver<Update>,
}
fn changed(previous: &[u8], bytes: &[u8], preview: bool) -> Result<Option<Catalog>> {
    if previous == bytes {
        return Ok(None);
    }
    let catalog: Catalog = serde_json::from_slice(bytes)?;
    catalog.validate(preview)?;
    Ok(Some(catalog))
}
pub fn worker(assets: PathBuf, preview: bool, initial: Vec<u8>) -> Worker {
    let (requests, queue) = mpsc::sync_channel(1);
    let (results, receiver) = mpsc::sync_channel(1);
    std::thread::spawn(move || {
        let mut previous = initial;
        let mut rejected = Vec::new();
        while queue.recv().is_ok() {
            let result = (|| -> Result<()> {
                let mut bytes = Vec::new();
                std::fs::File::open(assets.join("catalog.json"))?
                    .take(1024 * 1024 + 1)
                    .read_to_end(&mut bytes)?;
                anyhow::ensure!(bytes.len() <= 1024 * 1024, "Catalog exceeds its size limit");
                if bytes == rejected {
                    return Ok(());
                }
                let catalog = match changed(&previous, &bytes, preview) {
                    Ok(Some(catalog)) => catalog,
                    Ok(None) => return Ok(()),
                    Err(error) => {
                        rejected = bytes;
                        return Err(error);
                    }
                };
                let mut icons = vec![None; catalog.apps.len()];
                for (index, result) in render::decode_icons(&assets, &catalog.apps) {
                    match result {
                        Ok(icon) => icons[index] = Some(icon),
                        Err(error) => {
                            tracing::warn!(index, %error, "updated icon unavailable; using fallback")
                        }
                    }
                }
                results
                    .send(Update { catalog, icons })
                    .context("Home has closed")?;
                previous = bytes;
                rejected.clear();
                Ok(())
            })();
            if let Err(error) = result {
                tracing::warn!(%error, "catalog update ignored; keeping current apps");
            }
        }
    });
    Worker {
        requests,
        results: receiver,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn artwork_tracks_ids_after_moving_pins_and_refreshing_the_source() {
        let mut source: Catalog = serde_json::from_slice(&data("a", "a.png")).unwrap();
        let mut b = source.apps[0].clone();
        b.id = "b".into();
        source.apps.push(b);
        let mut displayed = source.clone();
        displayed.apps.reverse();
        let mut pin = source.apps[0].clone();
        pin.id = "hdmi".into();
        pin.input = true;
        displayed.apps.insert(1, pin);
        assert_eq!(
            icons_for(&source, &[Some("a icon"), Some("b icon")], &displayed),
            vec![Some("b icon"), None, Some("a icon")]
        );
        source.apps.reverse();
        assert_eq!(
            icons_for(&source, &[Some("updated b"), Some("updated a")], &displayed),
            vec![Some("updated b"), None, Some("updated a")]
        );
    }
    fn data(id: &str, artwork: &str) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({"preview":false,"apps":[{
            "id":id,"title":"App","artwork":artwork,"contain":true,"color":[0,0,0]
        }],"inputs":[]}))
        .unwrap()
    }
    #[test]
    fn unchanged_invalid_preview_and_unsafe_updates_leave_current_catalog_alone() {
        let initial = data("a", "icon.png");
        assert!(changed(&initial, &initial, false).unwrap().is_none());
        assert!(changed(&initial, b"{partial", false).is_err());
        assert!(changed(&initial, &data("preview.a", "icon.png"), false).is_err());
        assert!(changed(&initial, &data("b", "../../etc/passwd"), false).is_err());
        let updated = changed(&initial, &data("b", "tvhome-new.png"), false)
            .unwrap()
            .unwrap();
        assert_eq!(updated.apps[0].id, "b");
        assert!(changed(&initial, &data("b", ""), false).is_ok());
    }
}
