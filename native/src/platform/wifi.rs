//! Read-only status polling on a worker. Never scan or change network settings.
use super::luna;
use anyhow::Result;
use serde_json::Value;
use std::ffi::CStr;
use std::sync::mpsc;
use std::time::{Duration, Instant};

const SERVICE: &CStr = c"luna://com.webos.service.wifi/getstatus";
const REQUEST: &str = r#"{"subscribe":false}"#;
const INTERVAL: Duration = Duration::from_secs(10);
const MAX_AGE: Duration = Duration::from_secs(30);

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Connection {
    #[default]
    Unknown,
    Connected,
    Connecting,
    Disconnected,
    Disabled,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Status {
    pub connection: Connection,
    pub bars: Option<u8>,
    pub level: Option<i64>,
    pub name: Option<String>,
    pub ip: Option<String>,
    pub gateway: Option<String>,
    pub preview: bool,
}

impl Status {
    pub fn description(&self) -> &'static str {
        match self.connection {
            Connection::Unknown => "Status unavailable",
            Connection::Connected => "Connected",
            Connection::Connecting => "Connecting",
            Connection::Disconnected => "Not connected",
            Connection::Disabled => "Wi-Fi is off",
        }
    }

    pub fn signal_label(&self) -> String {
        match (self.bars, self.level) {
            (Some(bars), Some(level)) => format!(
                "{bars} {} · Level {level}",
                if bars == 1 { "bar" } else { "bars" }
            ),
            (Some(bars), None) => format!("{bars} {}", if bars == 1 { "bar" } else { "bars" }),
            (None, Some(level)) => format!("Level {level}"),
            _ => "Unavailable".into(),
        }
    }
}

fn field(value: &Value, key: &str) -> Option<String> {
    value.get(key)?.as_str().filter(|s| !s.is_empty()).map(|s| {
        // Keep service strings bounded, printable and inside the details panel.
        s.chars().filter(|c| !c.is_control()).take(128).collect()
    })
}

fn parse(payload: &str) -> Status {
    let Ok(value) = serde_json::from_str::<Value>(payload) else {
        return Status::default();
    };
    if value["returnValue"] != true {
        return Status::default();
    }
    // Wi-Fi Direct peers and scan results do not describe the current connection.
    let network = value
        .get("networkInfo")
        .or_else(|| value.get("wifi"))
        .unwrap_or(&value);
    let state = network
        .get("connectState")
        .or_else(|| network.get("state"))
        .and_then(Value::as_str);
    let connection = if value["status"] == "serviceDisabled" {
        Connection::Disabled
    } else {
        match state {
            Some("ipConfigured" | "connected") => Connection::Connected,
            Some("associating" | "associated") => Connection::Connecting,
            Some("notAssociated" | "disconnected" | "ipFailed") => Connection::Disconnected,
            None if value["status"] == "serviceEnabled" => Connection::Disconnected,
            _ => Connection::Unknown,
        }
    };
    if connection != Connection::Connected {
        return Status {
            connection,
            ..Status::default()
        };
    }
    let level = network["signalLevel"]
        .as_i64()
        .filter(|n| (0..=100).contains(n));
    // Map signalLevel to Home's three-bar scale independently of the TV's signalBars.
    let bars = level.map(|level| match level {
        0..=33 => 1,
        34..=65 => 2,
        _ => 3,
    });
    let ip_info = value
        .get("ipInfo")
        .or_else(|| network.get("ipInfo"))
        .unwrap_or(network);
    Status {
        connection,
        bars,
        level,
        name: field(network, "displayName").or_else(|| field(network, "ssid")),
        ip: field(ip_info, "ip").or_else(|| field(ip_info, "ipAddress")),
        gateway: field(ip_info, "gateway"),
        preview: false,
    }
}

fn run(preview: bool, mut request: impl FnMut(&CStr, &str) -> Result<String>) -> Status {
    if preview {
        if std::env::var_os("TVHOME_PREVIEW_WIFI_UNAVAILABLE").is_some() {
            return Status {
                preview: true,
                ..Status::default()
            };
        }
        // Explicit sample data, never a reading from the host or TV.
        let mut sample = parse(
            r#"{"returnValue":true,"networkInfo":{"connectState":"ipConfigured","displayName":"Example Wi-Fi","signalLevel":53,"signalBars":1},"ipInfo":{"ip":"192.0.2.10","gateway":"192.0.2.1"}}"#,
        );
        sample.preview = true;
        return sample;
    }
    match request(SERVICE, REQUEST) {
        Ok(payload) => parse(&payload),
        Err(_) => Status::default(),
    }
}

pub struct Monitor {
    pub status: Status,
    pending: bool,
    next_read: Instant,
    valid_until: Instant,
}

impl Monitor {
    pub fn new(now: Instant) -> Self {
        Self {
            status: Status::default(),
            pending: false,
            next_read: now,
            valid_until: now,
        }
    }

    pub fn refresh(&mut self, now: Instant) {
        self.next_read = now;
    }

    pub fn due(&self, now: Instant, foreground: bool) -> bool {
        foreground && !self.pending && now >= self.next_read
    }

    pub fn start(&mut self) {
        self.pending = true;
    }

    pub fn finish(&mut self, status: Status, now: Instant) -> bool {
        self.pending = false;
        self.next_read = now + INTERVAL;
        self.valid_until = now + MAX_AGE;
        let changed = self.status != status;
        self.status = status;
        changed
    }

    pub fn expire(&mut self, now: Instant) -> bool {
        if now >= self.valid_until && self.status != Status::default() {
            self.status = Status::default();
            true
        } else {
            false
        }
    }
}

pub struct Worker {
    pub requests: mpsc::SyncSender<()>,
    pub results: mpsc::Receiver<Status>,
}

pub fn worker(preview: bool) -> Worker {
    let (requests, queue) = mpsc::sync_channel::<()>(1);
    let (send, results) = mpsc::channel();
    std::thread::spawn(move || {
        for () in queue {
            // Network names/addresses and raw replies are never logged or persisted.
            if send.send(run(preview, luna::query)).is_err() {
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
    fn maps_signal_level_using_custom_thresholds_and_reads_network_details() {
        let reading = parse(
            r#"{"returnValue":true,"networkInfo":{"connectState":"ipConfigured","displayName":"Living room","ssid":"raw name","signalLevel":53,"signalBars":1},"ipInfo":{"ip":"192.0.2.10","gateway":"192.0.2.1"}}"#,
        );
        assert_eq!(reading.connection, Connection::Connected);
        assert_eq!(reading.bars, Some(2));
        assert_eq!(reading.signal_label(), "2 bars · Level 53");
        assert_eq!(reading.name.as_deref(), Some("Living room"));
        assert_eq!(reading.ip.as_deref(), Some("192.0.2.10"));
        assert_eq!(reading.gateway.as_deref(), Some("192.0.2.1"));
        for (level, bars) in [
            (0, 1),
            (33, 1),
            (34, 2),
            (53, 2),
            (65, 2),
            (66, 3),
            (100, 3),
        ] {
            let value = parse(&format!(
                r#"{{"returnValue":true,"networkInfo":{{"connectState":"ipConfigured","signalLevel":{level},"signalBars":0}}}}"#
            ));
            assert_eq!(value.bars, Some(bars));
        }
    }

    #[test]
    fn never_invents_signal_or_retains_a_disconnected_network() {
        for payload in [
            "{}",
            "broken",
            r#"{"returnValue":false,"networkInfo":{"connectState":"ipConfigured","signalBars":3}}"#,
            r#"{"returnValue":true,"wifi":{"state":"connected"},"wifiDirect":{"signalBars":3}}"#,
            r#"{"returnValue":true,"networkInfo":{"connectState":"ipConfigured","signalBars":3,"signalLevel":101}}"#,
            r#"{"returnValue":true,"networkInfo":{"connectState":"ipConfigured","signalBars":3,"signalLevel":"53"}}"#,
            r#"{"returnValue":true,"networkInfo":{"connectState":"ipConfigured","signalLevel":-1}}"#,
        ] {
            assert_eq!(parse(payload).bars, None);
        }
        for (state, expected) in [
            ("notAssociated", Connection::Disconnected),
            ("associating", Connection::Connecting),
            ("associated", Connection::Connecting),
        ] {
            let reading = parse(&format!(
                r#"{{"returnValue":true,"networkInfo":{{"connectState":"{state}","signalBars":3,"displayName":"Old network"}}}}"#
            ));
            assert_eq!(reading.connection, expected);
            assert_eq!(reading.bars, None);
            assert_eq!(reading.name, None);
        }
        assert_eq!(
            parse(r#"{"returnValue":true,"status":"serviceDisabled"}"#).connection,
            Connection::Disabled
        );
        assert_eq!(
            parse(r#"{"returnValue":true,"status":"serviceEnabled"}"#).connection,
            Connection::Disconnected
        );
    }

    #[test]
    fn only_reads_status_and_preview_never_contacts_tv() {
        let mut calls = Vec::new();
        let status = run(false, |uri, payload| {
            calls.push((uri.to_string_lossy().into_owned(), payload.to_owned()));
            anyhow::bail!("Access denied")
        });
        assert_eq!(
            calls,
            vec![(
                "luna://com.webos.service.wifi/getstatus".into(),
                r#"{"subscribe":false}"#.into()
            )]
        );
        assert_eq!(status, Status::default());
        let sample = run(true, |_, _| panic!("Preview must not call Luna"));
        assert!(sample.preview);
        assert_eq!(sample.bars, Some(2));
    }

    #[test]
    fn polling_pauses_offscreen_never_overlaps_and_discards_stale_readings() {
        let now = Instant::now();
        let mut monitor = Monitor::new(now);
        assert!(!monitor.due(now, false));
        assert!(monitor.due(now, true));
        monitor.start();
        assert!(!monitor.due(now + INTERVAL, true));
        let sample = run(true, |_, _| unreachable!());
        assert!(monitor.finish(sample.clone(), now));
        assert!(!monitor.due(now + Duration::from_secs(9), true));
        assert!(monitor.due(now + INTERVAL, true));
        assert!(!monitor.due(now + INTERVAL, false));
        monitor.refresh(now);
        assert!(monitor.due(now, true));
        assert!(!monitor.finish(sample, now));
        assert!(!monitor.expire(now + MAX_AGE - Duration::from_secs(1)));
        assert!(monitor.expire(now + MAX_AGE));
        assert_eq!(monitor.status, Status::default());
        monitor.start();
        monitor.finish(Status::default(), now + MAX_AGE);
        assert!(monitor.due(now + MAX_AGE + INTERVAL, true));
    }
}
