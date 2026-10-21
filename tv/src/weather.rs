use anyhow::{ensure, Context, Result};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::mpsc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const REFRESH: Duration = Duration::from_secs(15 * 60);
const RETRY: Duration = Duration::from_secs(5 * 60);
const MAX_AGE: i64 = 2 * 60 * 60;

fn timestamp() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct Location {
    pub city: String,
    pub latitude: f64,
    pub longitude: f64,
    #[serde(default)]
    pub region: String,
    #[serde(default)]
    pub country: String,
}
impl Location {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            !self.city.trim().is_empty()
                && [&self.city, &self.region, &self.country]
                    .iter()
                    .all(|s| { s.chars().count() <= 80 && !s.chars().any(char::is_control) })
                && (-90.0..=90.0).contains(&self.latitude)
                && (-180.0..=180.0).contains(&self.longitude),
            "Invalid location"
        );
        Ok(())
    }
    pub fn detail(&self) -> String {
        [&self.region, &self.country]
            .into_iter()
            .filter(|s| !s.is_empty())
            .map(String::as_str)
            .collect::<Vec<_>>()
            .join(", ")
    }
}

fn location(value: Value) -> Result<Location> {
    ensure!(value["success"] == true, "Location unavailable");
    let mut result: Location = serde_json::from_value(value)?;
    result.city = result
        .city
        .chars()
        .filter(|c| !c.is_control())
        .take(80)
        .collect();
    result.validate()?;
    Ok(result)
}

#[derive(Clone, Debug)]
pub struct Conditions {
    pub time: i64,
    pub temperature: f64,
    pub feels_like: Option<f64>,
    pub high: Option<f64>,
    pub low: Option<f64>,
    pub code: u32,
    pub day: bool,
}
impl Conditions {
    pub fn description(&self) -> &'static str {
        match self.code {
            0 => {
                if self.day {
                    "Clear sky"
                } else {
                    "Clear night"
                }
            }
            1 => "Mostly clear",
            2 => "Partly cloudy",
            3 => "Overcast",
            45 | 48 => "Fog",
            51..=57 => "Drizzle",
            61..=67 => "Rain",
            71..=77 => "Snow",
            80..=82 => "Rain showers",
            85 | 86 => "Snow showers",
            95..=99 => "Thunderstorms",
            _ => "Conditions unavailable",
        }
    }
    pub fn icon(&self) -> &'static str {
        match self.code {
            0 | 1 => {
                if self.day {
                    "sun"
                } else {
                    "moon"
                }
            }
            2 => {
                if self.day {
                    "cloud-sun"
                } else {
                    "cloud-moon"
                }
            }
            3 => "cloud",
            45 | 48 => "cloud-fog",
            51..=67 | 80..=82 => "cloud-rain",
            71..=77 | 85 | 86 => "cloud-snow",
            95..=99 => "cloud-lightning",
            _ => "cloud",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Air {
    pub time: i64,
    /// 401 represents the open-ended severe band, displayed as 400+.
    pub index: u16,
    pub pollutant: &'static str,
}
impl Air {
    pub fn label(&self) -> &'static str {
        match self.index {
            0..=50 => "Good",
            51..=100 => "Satisfactory",
            101..=200 => "Moderate",
            201..=300 => "Poor",
            301..=400 => "Very poor",
            _ => "Severe",
        }
    }
    pub fn number(&self) -> String {
        if self.index > 400 {
            "400+".into()
        } else {
            self.index.to_string()
        }
    }
    pub fn color(&self) -> [u8; 3] {
        match self.index {
            0..=50 => [163, 214, 175],
            51..=100 => [189, 214, 151],
            101..=200 => [232, 209, 141],
            201..=300 => [232, 171, 130],
            301..=400 => [235, 145, 145],
            _ => [225, 136, 179],
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct State {
    pub city: Option<String>,
    pub conditions: Option<Conditions>,
    pub air: Option<Air>,
    pub refreshing: bool,
    pub problem: Option<&'static str>,
    pub sample: bool,
}
impl State {
    fn expire(&mut self, now: i64) -> bool {
        let weather_old = self
            .conditions
            .as_ref()
            .is_some_and(|v| !fresh(v.time, now));
        let air_old = self.air.as_ref().is_some_and(|v| !fresh(v.time, now));
        if weather_old {
            self.conditions = None;
        }
        if air_old {
            self.air = None;
        }
        if weather_old || air_old {
            self.problem = Some("Readings expired. Waiting for an update.");
        }
        weather_old || air_old
    }
    pub fn age_label(&self) -> String {
        if self.sample {
            return "Sample readings".into();
        }
        if self.refreshing {
            return "Updating…".into();
        }
        if self.problem.is_some() {
            return "Update unavailable".into();
        }
        match self
            .conditions
            .as_ref()
            .map(|c| c.time)
            .into_iter()
            .chain(self.air.as_ref().map(|a| a.time))
            .min()
        {
            Some(at) => format!("Readings {} min ago", (timestamp() - at).max(0) / 60),
            None => "Waiting for readings".into(),
        }
    }
}
fn fresh(at: i64, now: i64) -> bool {
    at <= now + 300 && now.saturating_sub(at) <= MAX_AGE
}
fn number(value: &Value, min: f64, max: f64) -> Option<f64> {
    value
        .as_f64()
        .filter(|v| v.is_finite() && (min..=max).contains(v))
}
fn conditions(value: &Value, now: i64) -> Result<Conditions> {
    let current = &value["current"];
    let time = current["time"].as_i64().context("Weather time missing")?;
    ensure!(fresh(time, now), "Weather expired");
    let temperature =
        number(&current["temperature_2m"], -100.0, 70.0).context("Temperature missing")?;
    Ok(Conditions {
        time,
        temperature,
        feels_like: number(&current["apparent_temperature"], -120.0, 90.0),
        high: number(&value["daily"]["temperature_2m_max"][0], -100.0, 70.0),
        low: number(&value["daily"]["temperature_2m_min"][0], -100.0, 70.0),
        code: current["weather_code"]
            .as_u64()
            .filter(|c| *c <= 99)
            .unwrap_or(100) as u32,
        day: current["is_day"] == 1,
    })
}

// CPCB National AQI report, table 3.11: 24h concentrations, CO/O3 8h.
// Continuous interpolation between band endpoints; severe has no finite upper
// concentration endpoint, so report 400+ instead of inventing precision.
fn subindex(value: f64, breaks: [f64; 5]) -> u16 {
    let indices = [50.0, 100.0, 200.0, 300.0, 400.0];
    let (mut low, mut index_low) = (0.0, 0.0);
    for (high, index_high) in breaks.into_iter().zip(indices) {
        if value <= high {
            return (index_low + (value - low) * (index_high - index_low) / (high - low)).round()
                as u16;
        }
        (low, index_low) = (high, index_high);
    }
    401
}
fn air(value: &Value, now: i64) -> Result<Air> {
    let hourly = &value["hourly"];
    let times = hourly["time"]
        .as_array()
        .context("Air quality times missing")?;
    let times: Vec<i64> = times
        .iter()
        .map(|v| v.as_i64().context("Invalid air quality time"))
        .collect::<Result<_>>()?;
    ensure!(
        times.windows(2).all(|w| w[1] - w[0] == 3600),
        "Invalid hourly spacing"
    );
    let at = *times
        .iter()
        .filter(|t| **t <= now)
        .max()
        .context("No current air quality")?;
    ensure!(fresh(at, now), "Air quality expired");
    let average = |name: &str, hours: i64, minimum: usize| -> Option<f64> {
        let readings = hourly[name].as_array()?;
        if readings.len() != times.len() {
            return None;
        }
        let valid: Vec<f64> = times
            .iter()
            .zip(readings)
            .filter(|(t, _)| **t <= at && **t > at - hours * 3600)
            .filter_map(|(_, v)| number(v, 0.0, 1_000_000.0))
            .collect();
        (valid.len() >= minimum).then(|| valid.iter().sum::<f64>() / valid.len() as f64)
    };
    let mut indices = Vec::new();
    for (field, label, breaks) in [
        ("pm2_5", "PM2.5", [30.0, 60.0, 90.0, 120.0, 250.0]),
        ("pm10", "PM10", [50.0, 100.0, 250.0, 350.0, 430.0]),
        ("nitrogen_dioxide", "NO₂", [40.0, 80.0, 180.0, 280.0, 400.0]),
        ("sulphur_dioxide", "SO₂", [40.0, 80.0, 380.0, 800.0, 1600.0]),
    ] {
        if let Some(v) = average(field, 24, 16) {
            indices.push((subindex(v, breaks), label));
        }
    }
    if let Some(v) = average("carbon_monoxide", 8, 8) {
        indices.push((subindex(v / 1000.0, [1.0, 2.0, 10.0, 17.0, 34.0]), "CO"));
    }
    if let Some(v) = average("ozone", 8, 8) {
        // CPCB specifies one-hour ozone for the top two bands.
        let hourly = average("ozone", 1, 1).unwrap_or(0.0);
        let index = if hourly > 208.0 {
            subindex(hourly, [50.0, 100.0, 168.0, 208.0, 748.0])
        } else {
            subindex(v, [50.0, 100.0, 168.0, 208.0, 748.0]).min(300)
        };
        indices.push((index, "O₃"));
    }
    ensure!(
        indices.len() >= 3
            && indices
                .iter()
                .any(|(_, name)| matches!(*name, "PM2.5" | "PM10")),
        "Insufficient air quality data"
    );
    let (index, pollutant) = indices.into_iter().max_by_key(|v| v.0).unwrap();
    Ok(Air {
        time: at,
        index,
        pollutant,
    })
}

pub(crate) async fn json(client: &Client, url: &str, params: &[(&str, String)]) -> Result<Value> {
    let mut reply = client
        .get(url)
        .query(params)
        .send()
        .await?
        .error_for_status()?;
    let mut bytes = Vec::new();
    while let Some(chunk) = reply.chunk().await? {
        ensure!(
            bytes.len() + chunk.len() <= 256 * 1024,
            "Response too large"
        );
        bytes.extend_from_slice(&chunk);
    }
    Ok(serde_json::from_slice(&bytes)?)
}

async fn read(
    client: &Client,
    cached: &mut Option<(Location, Instant)>,
    selected: Option<&Location>,
) -> Result<State> {
    if selected.is_none()
        && cached
            .as_ref()
            .is_none_or(|(_, at)| at.elapsed() > Duration::from_secs(6 * 3600))
    {
        let place = location(
            json(
                client,
                "https://ipwho.is/",
                &[("fields", "success,city,latitude,longitude".into())],
            )
            .await?,
        )?;
        *cached = Some((place, Instant::now()));
    }
    let place = selected.unwrap_or_else(|| &cached.as_ref().unwrap().0);
    let base = [
        ("latitude", place.latitude.to_string()),
        ("longitude", place.longitude.to_string()),
        ("timeformat", "unixtime".into()),
    ];
    let mut weather_params = base.to_vec();
    weather_params.extend([
        (
            "current",
            "temperature_2m,apparent_temperature,weather_code,is_day".into(),
        ),
        ("daily", "temperature_2m_max,temperature_2m_min".into()),
        ("forecast_days", "1".into()),
        ("timezone", "auto".into()),
    ]);
    let mut air_params = base.to_vec();
    air_params.extend([
        (
            "hourly",
            "pm2_5,pm10,nitrogen_dioxide,sulphur_dioxide,carbon_monoxide,ozone".into(),
        ),
        ("past_days", "1".into()),
        ("forecast_days", "1".into()),
    ]);
    let (weather, quality) = tokio::join!(
        json(
            client,
            "https://api.open-meteo.com/v1/forecast",
            &weather_params
        ),
        json(
            client,
            "https://air-quality-api.open-meteo.com/v1/air-quality",
            &air_params
        )
    );
    let now = timestamp();
    let conditions = weather.and_then(|v| conditions(&v, now)).ok();
    let air = quality.and_then(|v| air(&v, now)).ok();
    let problem = match (conditions.is_some(), air.is_some()) {
        (true, true) => None,
        (true, false) => Some("Air quality is unavailable."),
        (false, true) => Some("Weather is unavailable."),
        (false, false) => Some("Weather and air quality are unavailable."),
    };
    Ok(State {
        city: Some(place.city.clone()),
        conditions,
        air,
        problem,
        ..State::default()
    })
}

fn sample() -> State {
    State {
        city: Some("Mumbai".into()),
        sample: true,
        conditions: Some(Conditions {
            time: timestamp(),
            temperature: 27.0,
            feels_like: Some(29.0),
            high: Some(29.0),
            low: Some(24.0),
            code: 2,
            day: true,
        }),
        air: Some(Air {
            time: timestamp(),
            index: 42,
            pollutant: "PM2.5",
        }),
        ..State::default()
    }
}

pub struct Monitor {
    pub state: State,
    next: Instant,
    started: Option<Instant>,
    selected: Option<Location>,
    generation: u64,
    in_flight: bool,
    requests: mpsc::SyncSender<(u64, Option<Location>)>,
    results: mpsc::Receiver<(u64, Result<State>)>,
}
impl Monitor {
    pub fn new(preview: bool, selected: Option<Location>) -> Self {
        let (requests, queue) = mpsc::sync_channel::<(u64, Option<Location>)>(1);
        let (send, results) = mpsc::sync_channel(1);
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
            let mut cached = None;
            for (generation, selected) in queue {
                let fixture = preview
                    .then(|| std::env::var("TVHOME_PREVIEW_WEATHER").ok())
                    .flatten();
                let result = match fixture.as_deref() {
                    Some("sample") => {
                        let mut state = sample();
                        if let Some(place) = &selected {
                            state.city = Some(place.city.clone());
                        }
                        Ok(state)
                    }
                    Some("unavailable") => Err(anyhow::anyhow!("Preview unavailable fixture")),
                    _ => match (&runtime, &client) {
                        (Ok(runtime), Ok(client)) => runtime.block_on(async {
                            tokio::time::timeout(
                                Duration::from_secs(25),
                                read(client, &mut cached, selected.as_ref()),
                            )
                            .await
                            .context("Weather deadline")?
                        }),
                        _ => Err(anyhow::anyhow!("Weather worker unavailable")),
                    },
                };
                if send.send((generation, result)).is_err() {
                    break;
                }
            }
        });
        Self {
            state: State {
                city: selected.as_ref().map(|p| p.city.clone()),
                ..State::default()
            },
            selected,
            generation: 0,
            in_flight: false,
            next: Instant::now(),
            started: None,
            requests,
            results,
        }
    }
    pub fn select(&mut self, selected: Option<Location>) {
        if self.selected == selected {
            return;
        }
        self.generation = self.generation.wrapping_add(1);
        self.state = State {
            city: selected.as_ref().map(|p| p.city.clone()),
            ..State::default()
        };
        self.selected = selected;
        self.next = Instant::now();
        self.started = None;
    }
    pub fn refresh(&mut self) {
        if self
            .started
            .is_none_or(|at| at.elapsed() >= Duration::from_secs(30))
        {
            self.next = Instant::now();
        }
    }
    pub fn tick(&mut self, foreground: bool) -> bool {
        let mut changed = self.state.expire(timestamp());
        for (generation, result) in self.results.try_iter() {
            self.in_flight = false;
            if generation != self.generation {
                continue;
            }
            let success = match result {
                Ok(state) => {
                    let success = state.problem.is_none();
                    self.state = state;
                    success
                }
                Err(_) => {
                    self.state.problem =
                        Some("Could not update. Check your connection and try again.");
                    false
                }
            };
            self.state.refreshing = false;
            self.next = Instant::now() + if success { REFRESH } else { RETRY };
            changed = true;
        }
        if foreground && !self.in_flight && Instant::now() >= self.next {
            if self
                .requests
                .try_send((self.generation, self.selected.clone()))
                .is_ok()
            {
                self.state.refreshing = true;
                self.in_flight = true;
                self.started = Some(Instant::now());
            } else {
                self.next = Instant::now() + RETRY;
            }
            changed = true;
        }
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    #[ignore = "Contacts IPWhois and Open-Meteo; run explicitly for integration validation"]
    fn live_providers_return_current_conditions_and_indian_aqi() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let client = Client::builder()
            .https_only(true)
            .timeout(Duration::from_secs(12))
            .build()
            .unwrap();
        let state = runtime.block_on(read(&client, &mut None, None)).unwrap();
        assert!(state.problem.is_none(), "{:?}", state.problem);
        let weather = state.conditions.unwrap();
        let air = state.air.unwrap();
        println!(
            "{}: {:.0}°C, {}, Indian AQI {} ({})",
            state.city.unwrap(),
            weather.temperature,
            weather.description(),
            air.number(),
            air.label()
        );
    }
    fn hourly(now: i64) -> Value {
        let times: Vec<i64> = (0..25).map(|i| now - (23 - i) * 3600).collect();
        let mut value = json!({"hourly": {"time": times}});
        for name in [
            "pm2_5",
            "pm10",
            "nitrogen_dioxide",
            "sulphur_dioxide",
            "ozone",
            "carbon_monoxide",
        ] {
            value["hourly"][name] = json!(vec![30.0; 25]);
        }
        value
    }
    #[test]
    fn indian_breakpoints_and_categories() {
        let breaks = [30.0, 60.0, 90.0, 120.0, 250.0];
        for (value, expected) in [
            (0.0, 0),
            (30.0, 50),
            (60.0, 100),
            (75.0, 150),
            (90.0, 200),
            (120.0, 300),
            (250.0, 400),
            (251.0, 401),
        ] {
            assert_eq!(subindex(value, breaks), expected);
        }
        let mut a = sample().air.unwrap();
        a.index = 401;
        assert_eq!(a.number(), "400+");
        assert_eq!(a.label(), "Severe");
    }

    #[test]
    fn poller_waits_in_background_bounds_requests_and_retries_failures() {
        let (requests, queue) = mpsc::sync_channel(1);
        let (send, results) = mpsc::sync_channel(1);
        let mut monitor = Monitor {
            selected: None,
            generation: 0,
            in_flight: false,
            state: State::default(),
            next: Instant::now(),
            started: None,
            requests,
            results,
        };
        monitor.tick(false);
        assert!(queue.try_recv().is_err());
        assert!(monitor.tick(true));
        queue.try_recv().unwrap();
        monitor.tick(true);
        assert!(queue.try_recv().is_err());
        send.send((0, Ok(sample()))).unwrap();
        monitor.tick(true);
        assert!(!monitor.state.refreshing);
        assert!(monitor.next > Instant::now() + Duration::from_secs(14 * 60));
        monitor.refresh(); // Rapid refresh is rate-limited.
        monitor.tick(true);
        assert!(queue.try_recv().is_err());
        send.send((0, Err(anyhow::anyhow!("offline")))).unwrap();
        monitor.tick(false);
        assert!(monitor.state.conditions.is_some());
        assert!(monitor.state.problem.is_some());
        assert!(monitor.next > Instant::now() + Duration::from_secs(4 * 60));
    }
    #[test]
    fn changing_city_discards_in_flight_readings_and_automatic_clears_the_override() {
        let (requests, queue) = mpsc::sync_channel(1);
        let (send, results) = mpsc::sync_channel(1);
        let mut monitor = Monitor {
            selected: None,
            generation: 0,
            in_flight: false,
            state: sample(),
            next: Instant::now(),
            started: None,
            requests,
            results,
        };
        monitor.tick(true);
        let (old, place) = queue.try_recv().unwrap();
        assert!(place.is_none());
        let place = Location {
            city: "Paris".into(),
            latitude: 48.85,
            longitude: 2.35,
            region: "Île-de-France".into(),
            country: "France".into(),
        };
        monitor.select(Some(place.clone()));
        assert!(monitor.state.conditions.is_none());
        assert!(monitor.state.air.is_none());
        monitor.tick(true);
        assert!(queue.try_recv().is_err()); // Only one weather request may run at once.
        send.send((old, Ok(sample()))).unwrap();
        monitor.tick(true);
        let (current, requested) = queue.try_recv().unwrap();
        assert_ne!(old, current);
        assert_eq!(requested, Some(place));
        assert_eq!(monitor.state.city.as_deref(), Some("Paris"));
        assert!(monitor.state.conditions.is_none());
        let mut updated = sample();
        updated.city = Some("Paris".into());
        send.send((current, Ok(updated))).unwrap();
        monitor.tick(true);
        assert!(monitor.state.conditions.is_some());
        monitor.select(None);
        assert!(monitor.state.city.is_none());
        assert!(monitor.state.conditions.is_none());
        monitor.tick(true);
        assert!(queue.try_recv().unwrap().1.is_none());
    }
    #[test]
    fn aqi_uses_past_averages_minimum_coverage_and_co_units() {
        let now = 100 * 3600;
        let mut v = hourly(now);
        v["hourly"]["pm2_5"][24] = json!(1000); // Future data cannot raise current AQI.
        assert_eq!(air(&v, now).unwrap().index, 50);
        v["hourly"]["carbon_monoxide"] = json!(vec![2000.0; 25]);
        assert_eq!(air(&v, now).unwrap().index, 100);
        for i in 0..9 {
            v["hourly"]["pm2_5"][i] = Value::Null;
        }
        v["hourly"]["pm10"] = Value::Null;
        assert!(air(&v, now).is_err());
        v["hourly"]["pm2_5"][8] = json!(30);
        assert_eq!(air(&v, now).unwrap().index, 100);
        v["hourly"]["time"][1] = v["hourly"]["time"][0].clone();
        assert!(air(&v, now).is_err());
    }
    #[test]
    fn stale_future_null_and_partial_weather() {
        let now = timestamp();
        let v = json!({"current":{"time":now,"temperature_2m":27,"weather_code":2,"is_day":1}});
        assert_eq!(conditions(&v, now).unwrap().description(), "Partly cloudy");
        assert!(conditions(&v, now + MAX_AGE + 1).is_err());
        assert!(conditions(&v, now - 301).is_err());
        assert!(conditions(&json!({"current":{"time":now,"temperature_2m":null}}), now).is_err());
        let mut state = sample();
        assert!(state.expire(now + MAX_AGE + 1));
        assert!(state.air.is_none() && state.conditions.is_none());
        assert!(location(json!({"success":false})).is_err());
        assert!(location(json!({"success":true,"city":"x","latitude":91,"longitude":0})).is_err());
    }
}
