use crate::{
    model::Direction,
    weather::{self, Location},
};
use anyhow::{ensure, Result};
use reqwest::Client;
use serde_json::Value;
use std::{sync::mpsc, time::Duration};

pub const KEYS: &str = "QWERTYUIOPASDFGHJKL'ZXCVBNM-,.";
pub const SPACE: usize = 30;
pub const DELETE: usize = 31;
pub const CLEAR: usize = 32;
pub const SEARCH: usize = 33;
pub const AUTO: usize = 34;
pub const BACK: usize = 35;
pub const RESULTS: usize = 36;
pub const MAX_RESULTS: usize = 5;

pub fn bounds(index: usize) -> (f32, f32, f32, f32) {
    match index {
        0..=29 => (
            140.0 + (index % 10) as f32 * 72.0,
            372.0 + (index / 10) as f32 * 74.0,
            64.0,
            62.0,
        ),
        SPACE => (140.0, 606.0, 228.0, 68.0),
        DELETE => (380.0, 606.0, 140.0, 68.0),
        CLEAR => (532.0, 606.0, 140.0, 68.0),
        SEARCH => (684.0, 606.0, 168.0, 68.0),
        AUTO => (140.0, 744.0, 340.0, 68.0),
        BACK => (512.0, 744.0, 340.0, 68.0),
        _ => (920.0, 348.0 + (index - RESULTS) as f32 * 96.0, 820.0, 84.0),
    }
}

pub fn navigate(index: usize, direction: Direction, count: usize) -> usize {
    let result = if count > 0 { RESULTS } else { index };
    match (index, direction) {
        (0..=29, Direction::Up) => {
            if index >= 10 {
                index - 10
            } else {
                index
            }
        }
        (0..=19, Direction::Down) => index + 10,
        (20..=29, Direction::Down) => SPACE + (index % 10) * 4 / 10,
        (0..=29, Direction::Left) => {
            if !index.is_multiple_of(10) {
                index - 1
            } else {
                index
            }
        }
        (0..=29, Direction::Right) => {
            if index % 10 < 9 {
                index + 1
            } else {
                result
            }
        }
        (SPACE..=SEARCH, Direction::Up) => 20 + [1, 4, 6, 8][index - SPACE],
        (SPACE..=SEARCH, Direction::Down) => {
            if index < CLEAR {
                AUTO
            } else {
                BACK
            }
        }
        (SPACE..=SEARCH, Direction::Left) => index.saturating_sub(1).max(SPACE),
        (SPACE..=SEARCH, Direction::Right) => {
            if index < SEARCH {
                index + 1
            } else {
                result
            }
        }
        (AUTO | BACK, Direction::Up) => {
            if index == AUTO {
                SPACE
            } else {
                SEARCH
            }
        }
        (AUTO | BACK, Direction::Left) => AUTO,
        (AUTO | BACK, Direction::Right) => {
            if index == AUTO {
                BACK
            } else {
                result
            }
        }
        (RESULTS.., Direction::Up) => index.saturating_sub(1).max(RESULTS),
        (RESULTS.., Direction::Down) => (index + 1).min(RESULTS + count.saturating_sub(1)),
        (RESULTS.., Direction::Left) => SEARCH,
        _ => index,
    }
}

#[derive(Default)]
pub struct Picker {
    pub query: String,
    pub results: Vec<Location>,
    pub searching: bool,
    pub saving: bool,
    pub message: Option<&'static str>,
    generation: u64,
}
impl Picker {
    fn edited(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.results.clear();
        self.searching = false;
        self.message = None;
    }
    pub fn type_text(&mut self, text: &str) {
        if self.saving {
            return;
        }
        let remaining = 80usize.saturating_sub(self.query.chars().count());
        let text: String = text
            .chars()
            .filter(|c| !c.is_control())
            .take(remaining)
            .collect();
        if !text.is_empty() {
            self.query.push_str(&text);
            self.edited();
        }
    }
    pub fn delete(&mut self) {
        if !self.saving && self.query.pop().is_some() {
            self.edited();
        }
    }
    pub fn clear(&mut self) {
        if !self.saving {
            self.query.clear();
            self.edited();
        }
    }
    pub fn cancel(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.searching = false;
    }
    pub fn begin(&mut self) -> Option<(u64, String)> {
        if self.searching || self.saving {
            return None;
        }
        if self.query.trim().chars().count() < 2 {
            self.message = Some("Enter at least two letters.");
            return None;
        }
        self.generation = self.generation.wrapping_add(1);
        self.results.clear();
        self.message = None;
        self.searching = true;
        Some((self.generation, self.query.trim().to_owned()))
    }
    pub fn finish(&mut self, generation: u64, result: Result<Vec<Location>>) -> bool {
        if generation != self.generation {
            return false;
        }
        self.searching = false;
        match result {
            Ok(results) => {
                self.results = results;
                self.message = self
                    .results
                    .is_empty()
                    .then_some("No cities found. Try another name.");
            }
            Err(_) => self.message = Some("Search unavailable. Try again."),
        }
        true
    }
}

fn clean(value: &Value) -> String {
    value
        .as_str()
        .unwrap_or_default()
        .chars()
        .filter(|c| !c.is_control())
        .take(80)
        .collect::<String>()
        .trim()
        .to_owned()
}
fn parse(value: Value) -> Result<Vec<Location>> {
    ensure!(
        value.is_object() && value["error"] != true,
        "Invalid search response"
    );
    let Some(results) = value.get("results") else {
        return Ok(Vec::new());
    };
    let results = results
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("Invalid search results"))?;
    let mut locations = Vec::new();
    for value in results.iter().take(100) {
        let Some(latitude) = value["latitude"].as_f64() else {
            continue;
        };
        let Some(longitude) = value["longitude"].as_f64() else {
            continue;
        };
        let place = Location {
            city: clean(&value["name"]),
            latitude,
            longitude,
            region: clean(&value["admin1"]),
            country: clean(&value["country"]),
        };
        if place.validate().is_ok() && !locations.contains(&place) {
            locations.push(place);
        }
        if locations.len() == MAX_RESULTS {
            break;
        }
    }
    Ok(locations)
}
async fn search(client: &Client, query: &str) -> Result<Vec<Location>> {
    parse(
        weather::json(
            client,
            "https://geocoding-api.open-meteo.com/v1/search",
            &[
                ("name", query.to_owned()),
                ("count", MAX_RESULTS.to_string()),
                ("language", "en".into()),
            ],
        )
        .await?,
    )
}
pub struct Worker {
    pub requests: mpsc::SyncSender<(u64, String)>,
    pub results: mpsc::Receiver<(u64, Result<Vec<Location>>)>,
}
pub fn worker() -> Worker {
    let (requests, queue) = mpsc::sync_channel::<(u64, String)>(1);
    let (send, results) = mpsc::channel();
    std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build();
        let client = Client::builder()
            .https_only(true)
            .redirect(reqwest::redirect::Policy::none())
            .user_agent("tvhome/0.1")
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(12))
            .build();
        for (generation, query) in queue {
            let result = match (&runtime, &client) {
                (Ok(runtime), Ok(client)) => runtime.block_on(search(client, &query)),
                _ => Err(anyhow::anyhow!("City search unavailable")),
            };
            if send.send((generation, result)).is_err() {
                break;
            }
        }
    });
    Worker { requests, results }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn validates_results_and_disambiguates_same_named_cities() {
        let values = serde_json::json!({"results":[
            {"name":"Paris","admin1":"Île-de-France","country":"France","latitude":48.85,"longitude":2.35},
            {"name":"Paris","admin1":"Texas","country":"United States","latitude":33.66,"longitude":-95.55},
            {"name":"Bad","latitude":999,"longitude":0},
            {"name":"Missing coordinates"}
        ]});
        let cities = parse(values).unwrap();
        assert_eq!(cities.len(), 2);
        assert_eq!(cities[1].detail(), "Texas, United States");
        assert!(parse(serde_json::json!({})).unwrap().is_empty());
        assert!(parse(serde_json::json!({"error":true})).is_err());
    }
    #[test]
    fn edits_cancel_stale_searches_and_support_unicode_deletion() {
        let mut picker = Picker::default();
        picker.type_text("Par");
        let (old, _) = picker.begin().unwrap();
        assert!(picker.begin().is_none());
        picker.type_text("is");
        assert!(!picker.finish(old, Ok(Vec::new())));
        let (current, query) = picker.begin().unwrap();
        assert_eq!(query, "Paris");
        assert!(picker.finish(current, Ok(Vec::new())));
        assert!(picker.message.unwrap().starts_with("No cities"));
        picker.clear();
        picker.type_text("São");
        picker.delete();
        assert_eq!(picker.query, "Sã");
        picker.type_text(&"x".repeat(100));
        assert_eq!(picker.query.chars().count(), 80);
        let (old, _) = picker.begin().unwrap();
        picker.cancel();
        assert!(!picker.finish(old, Ok(Vec::new())));
    }
    #[test]
    fn remote_can_reach_search_results_auto_and_back() {
        assert_eq!(KEYS.len(), 30);
        assert_eq!(navigate(29, Direction::Down, 0), SEARCH);
        assert_eq!(navigate(SEARCH, Direction::Right, 2), RESULTS);
        assert_eq!(navigate(RESULTS, Direction::Down, 2), RESULTS + 1);
        assert_eq!(navigate(RESULTS + 1, Direction::Down, 2), RESULTS + 1);
        assert_eq!(navigate(RESULTS, Direction::Left, 2), SEARCH);
        assert_eq!(navigate(SEARCH, Direction::Down, 2), BACK);
        assert_eq!(navigate(BACK, Direction::Left, 2), AUTO);
    }
    #[test]
    #[ignore = "Contacts Open-Meteo geocoding; run explicitly for integration validation"]
    fn live_city_search() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let client = Client::builder()
            .https_only(true)
            .timeout(Duration::from_secs(12))
            .build()
            .unwrap();
        let cities = runtime.block_on(search(&client, "Mumbai")).unwrap();
        assert!(cities
            .iter()
            .any(|c| c.city == "Mumbai" && c.country == "India"));
    }
}
