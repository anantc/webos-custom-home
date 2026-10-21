use crate::{
    common::{fault, private_write, read_response, Config, Result},
    tv,
};
use reqwest::{redirect::Policy, Client, Url};
use serde_json::{json, Value};
use std::{fs, path::Path, time::Duration};

pub(crate) fn app_record(point: &Value, icon: &str) -> Option<Value> {
    if point["hidden"] == true {
        return None;
    }
    let id = point["id"].as_str().filter(|id| !id.is_empty())?;
    Some(
        json!({"id":id,"title":point["title"].as_str().unwrap_or(id),"icon":icon,"iconColor":point["iconColor"].as_str().filter(|s|!s.is_empty()).or(point["bgColor"].as_str()).unwrap_or("")}),
    )
}
pub(crate) fn inputs(devices: &[Value]) -> Vec<Value> {
    devices.iter().filter_map(|d|{let id=d["appId"].as_str().filter(|id|!id.is_empty())?;Some(json!({"appId":id,"label":d["label"].as_str().filter(|s|!s.is_empty()).or(d["id"].as_str()).unwrap_or(id)}))}).collect()
}
fn icon_url(point: &Value) -> Option<&str> {
    point["largeIcon"]
        .as_str()
        .filter(|url| url.starts_with("http://") || url.starts_with("https://"))
        .or(point["icon"].as_str())
}
pub(crate) async fn icon(config: &Config, point: &Value) -> Result<Vec<u8>> {
    let url = Url::parse(icon_url(point).ok_or(fault("No app artwork URL."))?)
        .map_err(|_| fault("Invalid artwork URL."))?;
    if !["http", "https"].contains(&url.scheme())
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err(fault("Invalid artwork URL."));
    }
    let local = url.host_str() == Some(config.host()?);
    // Only the configured TV gets the local-certificate exception; publisher HTTPS stays verified.
    let client = Client::builder()
        .redirect(Policy::none())
        .timeout(Duration::from_secs(10))
        .danger_accept_invalid_certs(local)
        .no_proxy()
        .build()
        .map_err(|_| fault("Cannot initialize artwork client."))?;
    read_response(
        client
            .get(url)
            .send()
            .await
            .map_err(|_| fault("Artwork download failed."))?,
        8 * 1024 * 1024,
    )
    .await
}
// State names: https://github.com/hobbyquaker/lgtv2#getpowerstatecallback--subscribepowerstatecallback
fn standby(power: &Value) -> bool {
    matches!(
        power["state"].as_str(),
        Some("Active Standby" | "Suspend" | "Power Off" | "Screen Off")
    )
}
pub async fn capture(config: &Config, pair: bool) -> Result<(Vec<Value>, Vec<Value>)> {
    let mut peer = tv::connect(config, pair).await?;
    let result = async {
        if !pair {
            if let Ok(power) = peer
                .request(
                    "ssap://com.webos.service.tvpower/power/getPowerState",
                    "power",
                )
                .await
            {
                if standby(&power) {
                    return Err(crate::common::Error::Unavailable);
                }
            }
        }
        let points = peer
            .request(
                "ssap://com.webos.applicationManager/listLaunchPoints",
                "apps",
            )
            .await?;
        let devices = peer
            .request("ssap://tv/getExternalInputList", "inputs")
            .await?;
        Ok::<_, crate::common::Error>((points, devices))
    }
    .await;
    peer.close().await;
    let (points, devices) = result?;
    let points = points["launchPoints"]
        .as_array()
        .ok_or(fault("TV did not return an app list."))?;
    let devices = devices["devices"]
        .as_array()
        .ok_or(fault("TV did not return an input list."))?;
    Ok((points.clone(), devices.clone()))
}
pub async fn run(config: &Config, directory: &Path, list_only: bool) -> Result<()> {
    let (points, devices) = capture(config, true).await?;
    if list_only {
        for point in &points {
            if let Some(app) = app_record(point, "") {
                println!(
                    "{:<45} {}",
                    app["id"].as_str().unwrap(),
                    app["title"].as_str().unwrap()
                );
            }
        }
        return Ok(());
    }
    let manifest = directory.join("tv.json");
    if manifest.exists() {
        let saved: Value = serde_json::from_slice(
            &fs::read(&manifest).map_err(|_| fault("Cannot read catalog TV identity."))?,
        )
        .map_err(|_| fault("Invalid catalog TV identity."))?;
        if saved["name"] != config.tv {
            return Err(fault(
                "Catalog directory belongs to another TV. Use this profile's default directory.",
            ));
        }
    }
    let icons = directory.join("icons");
    fs::create_dir_all(&icons).map_err(|_| fault("Cannot create catalog icon directory."))?;
    let mut apps = Vec::new();
    for point in &points {
        let Some(mut app) = app_record(point, "") else {
            continue;
        };
        let id = app["id"].as_str().unwrap();
        let name = format!(
            "{}.png",
            id.chars()
                .map(|c| if c.is_ascii_alphanumeric() || "._-".contains(c) {
                    c
                } else {
                    '_'
                })
                .collect::<String>()
        );
        match icon(config, point).await {
            Ok(bytes) if !bytes.is_empty() => {
                private_write(&icons.join(&name), &bytes)?;
                app["icon"] = format!("icons/{name}").into();
            }
            _ => crate::log(
                "An app icon was unavailable; keeping its real launch ID with a fallback icon.",
            ),
        }
        apps.push(app);
    }
    let inputs = inputs(&devices);
    let count = apps.len();
    let input_count = inputs.len();
    let data = serde_json::to_string_pretty(&json!({"apps":apps,"inputs":inputs}))
        .map_err(|_| fault("Cannot encode catalog."))?;
    private_write(
        &directory.join("apps.js"),
        format!("// Generated by tvhome: the TV's apps and inputs.\nwindow.TV_APPS = {data};\n")
            .as_bytes(),
    )?;
    private_write(
        &manifest,
        &serde_json::to_vec_pretty(&json!({"name":config.tv,"host":config.host()?}))
            .map_err(|_| fault("Cannot encode catalog TV identity."))?,
    )?;
    crate::log(&format!(
        "Synced {count} apps and {input_count} inputs for {}.",
        config.tv
    ));
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn standby_is_skipped_without_guessing_unknown_power_states() {
        for state in ["Active Standby", "Suspend", "Power Off", "Screen Off"] {
            assert!(standby(&json!({"state":state})));
        }
        for value in [
            json!({"state":"Active"}),
            json!({"state":"Screen Saver"}),
            json!({}),
        ] {
            assert!(!standby(&value));
        }
    }
    #[test]
    fn failed_icons_keep_real_ids_and_hidden_apps_stay_hidden() {
        let record = app_record(
            &json!({"id":"native.netflix","title":"Netflix","icon":"broken"}),
            "",
        )
        .unwrap();
        assert_eq!(record["id"], "native.netflix");
        assert_eq!(record["icon"], "");
        assert!(app_record(&json!({"id":"hidden","hidden":true}), "").is_none());
        assert_eq!(
            inputs(&[
                json!({"appId":"native.hdmi1","label":"Console"}),
                json!({"id":"bad"})
            ]),
            vec![json!({"appId":"native.hdmi1","label":"Console"})]
        );
    }
    #[test]
    fn large_icon_falls_back_to_normal_icon() {
        let app = json!({"largeIcon":"file://bad","icon":"http://tv/icon"});
        assert_eq!(icon_url(&app), Some("http://tv/icon"));
    }
}
