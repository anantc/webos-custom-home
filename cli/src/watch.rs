mod timing;
use timing::Timing;

use crate::{
    common::{fault, Config, Error, Http, Result},
    tv::{self, Event, Peer},
};
use serde_json::{json, Value};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{
    io::{AsyncRead, AsyncWrite},
    time::{sleep, Instant},
};
const LG_HOME: &str = "com.webos.app.home";

#[derive(Default)]
struct Redirect {
    previous: String,
    last: Option<Instant>,
}
impl Redirect {
    fn should_launch(&mut self, app: &str, now: Instant) -> bool {
        if app == self.previous {
            return false;
        }
        self.previous = app.into();
        if app != LG_HOME {
            return false;
        }
        if self
            .last
            .is_some_and(|t| now.duration_since(t) < Duration::from_secs(3))
        {
            self.last = None;
            crate::log("Double Home: staying on LG Home.");
            return false;
        }
        self.last = Some(now);
        true
    }
}
struct State {
    phase: &'static str,
    failure: Option<&'static str>,
    progress: Instant,
}
impl State {
    fn new() -> Self {
        Self {
            phase: "starting",
            failure: None,
            progress: Instant::now(),
        }
    }
    fn update(&mut self, phase: &'static str, failure: Option<&'static str>) {
        if self.phase != phase || self.failure != failure {
            crate::log(&format!("Watcher: {phase}"));
        }
        self.phase = phase;
        self.failure = failure;
        self.progress = Instant::now();
    }
    fn heartbeat(&self, now: Instant) -> (&'static str, &'static str) {
        if now.duration_since(self.progress) > Duration::from_secs(120) {
            return (
                "/fail",
                "Watcher made no progress for over two minutes. Check host logs.",
            );
        }
        if let Some(reason) = self.failure {
            return ("/fail", reason);
        }
        if self.phase == "watching" {
            ("", "Watcher subscribed and transport responsive.")
        } else {
            (
                "",
                "Watcher running; TV unavailable or in standby. Will reconnect.",
            )
        }
    }
}
struct Subscription {
    redirect: Redirect,
    subscribed: bool,
    deadline: Option<Instant>,
    pending: bool,
}
impl Subscription {
    fn new() -> Self {
        Self {
            redirect: Redirect::default(),
            subscribed: false,
            deadline: Some(Instant::now() + Duration::from_secs(10)),
            pending: false,
        }
    }
    fn receive(&mut self, message: &Value, now: Instant) -> Result<bool> {
        match message["id"].as_str() {
            Some("launch") if self.pending => {
                if tv::payload(message)?["returnValue"] != true {
                    return Err(fault("TV did not confirm the Home launch."));
                }
                self.pending = false;
                self.deadline = None;
            }
            Some("fg") => {
                let p = tv::payload(message)?;
                let app = p["appId"]
                    .as_str()
                    .ok_or(fault("Foreground subscription did not return an app ID."))?;
                if !self.subscribed {
                    self.subscribed = true;
                    self.deadline = None;
                }
                if self.redirect.should_launch(app, now) && !self.pending {
                    self.pending = true;
                    self.deadline = Some(now + Duration::from_secs(10));
                    return Ok(true);
                }
            }
            _ => {}
        }
        Ok(false)
    }
}
async fn watch_peer<S: AsyncRead + AsyncWrite + Unpin>(
    peer: &mut Peer<S>,
    config: &Config,
    state: &Arc<Mutex<State>>,
    timing: &mut Timing,
) -> Result<()> {
    let catalog_started = Instant::now();
    let points = peer
        .request(
            "ssap://com.webos.applicationManager/listLaunchPoints",
            "apps",
        )
        .await?;
    let points = points["launchPoints"]
        .as_array()
        .ok_or(fault("TV did not return installed launch points."))?;
    let target = config.launcher()?;
    if !points.iter().any(|p| p["id"] == target) {
        return Err(fault("Configured Home launcher is not installed."));
    }
    crate::log(&timing.event(&format!(
        "event=launcher_checked elapsed_ms={}",
        catalog_started.elapsed().as_millis()
    )));
    let subscribe_started = Instant::now();
    peer.send(json!({"type":"subscribe","id":"fg","uri":"ssap://com.webos.applicationManager/getForegroundAppInfo","payload":{}})).await?;
    let mut subscription = Subscription::new();
    let mut ping_at = Instant::now() + Duration::from_secs(20);
    let mut pong_deadline: Option<Instant> = None;
    let mut ping_sent: Option<Instant> = None;
    loop {
        let now = Instant::now();
        if let Some(line) = timing.expire(now) {
            crate::log(&line);
        }
        if subscription.deadline.is_some_and(|d| now >= d) {
            return Err(fault(
                "TV did not acknowledge the subscription or app launch.",
            ));
        }
        if pong_deadline.is_some_and(|d| now >= d) {
            crate::log(&timing.event("event=transport_timeout waiting_ms=10000"));
            // A TV entering standby may leave the socket open without answering pings.
            return Err(Error::Unavailable);
        }
        if now >= ping_at {
            ping_sent = Some(Instant::now());
            peer.ping().await?;
            pong_deadline = Some(Instant::now() + Duration::from_secs(10));
            ping_at = Instant::now() + Duration::from_secs(20);
        }
        let deadline = subscription
            .deadline
            .unwrap_or(ping_at)
            .min(pong_deadline.unwrap_or(ping_at))
            .min(ping_at)
            .min(timing.deadline().unwrap_or(ping_at));
        // Wake at the next protocol deadline even if no foreground changes occur.
        let event =
            match tokio::time::timeout_at(deadline, peer.next(deadline + Duration::from_secs(1)))
                .await
            {
                Err(_) => continue,
                Ok(result) => result?,
            };
        state.lock().unwrap().progress = Instant::now();
        match event {
            Event::Pong => {
                let received = Instant::now();
                pong_deadline = None;
                if let Some(sent) = ping_sent.take() {
                    crate::log(&timing.event(&format!(
                        "event=transport_pong rtt_ms={}",
                        received.duration_since(sent).as_millis()
                    )));
                }
            }
            Event::Json(message) => {
                let received = Instant::now();
                let was_subscribed = subscription.subscribed;
                let was_pending = subscription.pending;
                let launch = subscription.receive(&message, received)?;
                if launch {
                    let unfinished = timing.end("next_launch", received);
                    timing.begin_launch(received, !was_subscribed);
                    peer.send(json!({"type":"request","id":"launch","uri":"ssap://system.launcher/launch","payload":{"id":target}})).await?;
                    // Record dispatch after flushing the request, before any disk logging.
                    let sent = Instant::now();
                    crate::log(&timing.sent(sent));
                    if let Some(line) = unfinished {
                        crate::log(&line);
                    }
                }
                if !was_subscribed && subscription.subscribed {
                    crate::log(&timing.event(&format!(
                        "event=subscribed elapsed_ms={}",
                        received.duration_since(subscribe_started).as_millis()
                    )));
                    state.lock().unwrap().update("watching", None);
                }
                if message["id"] == "launch" && was_pending {
                    if let Some(line) = timing.acknowledged(received) {
                        crate::log(&line);
                    }
                }
                if message["id"] == "fg" && message["payload"]["appId"] == target {
                    if let Some(line) = timing.foreground(received) {
                        crate::log(&line);
                    }
                }
            }
        }
    }
}
async fn watch_loop(
    config: &Config,
    http: &Http,
    url: &str,
    state: Arc<Mutex<State>>,
) -> Result<()> {
    let mut connection = 0;
    let mut disconnected_at = Instant::now();
    loop {
        connection += 1;
        let mut timing = Timing::new(connection);
        let started = Instant::now();
        crate::log(&timing.event("event=connect_start"));
        state.lock().unwrap().progress = started;
        let result = match tv::connect(config, false).await {
            Ok(mut peer) => {
                let connected = Instant::now();
                crate::log(&timing.event(&format!(
                    "event=connected connect_ms={} reconnect_ms={}",
                    connected.duration_since(started).as_millis(),
                    connected.duration_since(disconnected_at).as_millis()
                )));
                let result = watch_peer(&mut peer, config, &state, &mut timing).await;
                disconnected_at = Instant::now();
                if let Some(line) = timing.end("connection_ended", disconnected_at) {
                    crate::log(&line);
                }
                crate::log(&timing.event(&format!(
                    "event=connection_ended connected_ms={}",
                    disconnected_at.duration_since(connected).as_millis()
                )));
                peer.close().await;
                result
            }
            Err(e) => {
                crate::log(&timing.event(&format!(
                    "event=connect_failed elapsed_ms={}",
                    started.elapsed().as_millis()
                )));
                Err(e)
            }
        };
        match result {
            Err(Error::Unavailable) => {
                let mut s = state.lock().unwrap();
                let failure = s.failure;
                s.update("TV unavailable", failure);
            }
            Err(e) => {
                crate::log(&e.to_string());
                let reason = match e {
                    Error::Fault(reason) => reason,
                    _ => "Watcher request failed. Check host logs.",
                };
                state
                    .lock()
                    .unwrap()
                    .update("TV request failed", Some(reason));
                http.ping(url, "/fail", reason).await;
            }
            Ok(()) => {}
        }
        crate::log(&timing.event("event=reconnect_wait delay_ms=20000"));
        sleep(Duration::from_secs(20)).await;
    }
}

async fn heartbeat_loop(
    http: &Http,
    url: &str,
    state: Arc<Mutex<State>>,
    catalog: Arc<Mutex<CatalogHealth>>,
) -> Result<()> {
    loop {
        sleep(Duration::from_secs(60)).await;
        let (mut suffix, mut message) = state.lock().unwrap().heartbeat(Instant::now());
        if catalog.lock().unwrap().failed(Instant::now()) {
            suffix = "/fail";
            message = "App catalog updates failed or stalled. Check host logs.";
        }
        http.ping(url, suffix, message).await;
    }
}

struct CatalogHealth {
    fault: bool,
    progress: Instant,
}
impl CatalogHealth {
    fn new() -> Self {
        Self {
            fault: false,
            progress: Instant::now(),
        }
    }
    fn failed(&self, now: Instant) -> bool {
        self.fault || now.duration_since(self.progress) > Duration::from_secs(300)
    }
    fn finish(&mut self, result: &Result<bool>) {
        self.progress = Instant::now();
        match result {
            Ok(_) => self.fault = false,
            Err(Error::Unavailable) => (), // Standby does not clear a previous publishing failure.
            Err(_) => self.fault = true,
        }
    }
}
async fn catalog_loop(
    config: &Config,
    template: &crate::package::Package,
    state: Arc<Mutex<CatalogHealth>>,
) -> Result<()> {
    loop {
        let started = Instant::now();
        let result = crate::inventory::update(config, template).await;
        let outcome = match &result {
            Ok(true) => "updated",
            Ok(false) => "unchanged",
            Err(Error::Unavailable) => "unavailable",
            Err(_) => "failed",
        };
        crate::log(&format!(
            "Catalog timing: result={outcome} elapsed_ms={}",
            started.elapsed().as_millis()
        ));
        if let Err(error) = &result {
            if *error != Error::Unavailable {
                crate::log(&format!("Catalog: {error}"));
            }
        }
        state.lock().unwrap().finish(&result);
        sleep(Duration::from_secs(60)).await;
    }
}

pub async fn run(config: &Config) -> Result<()> {
    config.host()?;
    config.launcher()?;
    if config.key().is_none() {
        return Err(fault("No pairing key. Run tvhome --tv NAME pair first."));
    }
    crate::log(&format!(
        "Watcher started: tv={} version={} pid={}",
        config.tv,
        env!("CARGO_PKG_VERSION"),
        std::process::id()
    ));
    let url = config.ping_url("HC_WATCHER_URL", false)?;
    let http = Http::new()?;
    let template = crate::package::Package::read(crate::package::bundled())?;
    crate::ssh::validate(config)?;
    let state = Arc::new(Mutex::new(State::new()));
    let catalog = Arc::new(Mutex::new(CatalogHealth::new()));
    tokio::try_join!(
        watch_loop(config, &http, &url, state.clone()),
        heartbeat_loop(&http, &url, state, catalog.clone()),
        catalog_loop(config, &template, catalog)
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn catalog_failures_survive_standby_until_successful_retry() {
        let mut state = CatalogHealth::new();
        state.finish(&Err(fault("upload failed")));
        assert!(state.failed(Instant::now()));
        state.finish(&Err(Error::Unavailable));
        assert!(state.failed(Instant::now()));
        state.finish(&Ok(false));
        assert!(!state.failed(Instant::now()));
        assert!(state.failed(state.progress + Duration::from_secs(301)));
    }
    #[tokio::test(start_paused = true)]
    async fn silent_standby_after_subscription_does_not_fail_healthcheck() {
        let (mut client, mut server) = tv::mock_pair().await;
        let config = crate::common::test_config(std::path::Path::new("/tmp"));
        let state = Arc::new(Mutex::new(State::new()));
        let mut timing = Timing::new(1);
        let simulation = tokio::spawn(async move {
            server
                .next(Instant::now() + Duration::from_secs(1))
                .await
                .unwrap();
            server.send(json!({"id":"apps","payload":{"launchPoints":[{"id":"com.anant.tvhome.native"}]}})).await.unwrap();
            server
                .next(Instant::now() + Duration::from_secs(1))
                .await
                .unwrap();
            server
                .send(json!({"id":"fg","payload":{"appId":"netflix"}}))
                .await
                .unwrap();
            // Keep the connection open but stop reading/responding, as standby did on the TV.
            std::future::pending::<()>().await;
        });
        let started = Instant::now();
        let result = watch_peer(&mut client, &config, &state, &mut timing).await;
        simulation.abort();
        assert_eq!(result, Err(Error::Unavailable));
        assert_eq!(started.elapsed(), Duration::from_secs(30));
        let state = state.lock().unwrap();
        assert_eq!(state.phase, "watching");
        assert_eq!(state.heartbeat(Instant::now()).0, "");
    }
    #[tokio::test(start_paused = true)]
    async fn missing_launch_ack_times_out_after_foreground_switch() {
        let (mut client, mut server) = tv::mock_pair().await;
        let c = crate::common::test_config(std::path::Path::new("/tmp"));
        let state = Arc::new(Mutex::new(State::new()));
        let mut timing = Timing::new(1);
        let simulation = tokio::spawn(async move {
            server
                .next(Instant::now() + Duration::from_secs(1))
                .await
                .unwrap();
            server.send(json!({"id":"apps","payload":{"launchPoints":[{"id":"com.anant.tvhome.native"}]}})).await.unwrap();
            server
                .next(Instant::now() + Duration::from_secs(1))
                .await
                .unwrap();
            server
                .send(json!({"id":"fg","payload":{"appId":LG_HOME}}))
                .await
                .unwrap();
            server
                .next(Instant::now() + Duration::from_secs(1))
                .await
                .unwrap();
            server
                .send(json!({"id":"fg","payload":{"appId":"com.anant.tvhome.native"}}))
                .await
                .unwrap();
            std::future::pending::<()>().await;
        });
        let result = watch_peer(&mut client, &c, &state, &mut timing).await;
        simulation.abort();
        assert_eq!(
            result,
            Err(fault(
                "TV did not acknowledge the subscription or app launch."
            ))
        );
    }
    #[tokio::test(start_paused = true)]
    async fn missing_foreground_timing_does_not_fail_an_acknowledged_launch() {
        let (mut client, mut server) = tv::mock_pair().await;
        let c = crate::common::test_config(std::path::Path::new("/tmp"));
        let state = Arc::new(Mutex::new(State::new()));
        let mut timing = Timing::new(1);
        let simulation = async {
            server
                .next(Instant::now() + Duration::from_secs(1))
                .await
                .unwrap();
            server.send(json!({"id":"apps","payload":{"launchPoints":[{"id":"com.anant.tvhome.native"}]}})).await.unwrap();
            server
                .next(Instant::now() + Duration::from_secs(1))
                .await
                .unwrap();
            server
                .send(json!({"id":"fg","payload":{"appId":LG_HOME}}))
                .await
                .unwrap();
            server
                .next(Instant::now() + Duration::from_secs(1))
                .await
                .unwrap();
            server
                .send(json!({"id":"launch","payload":{"returnValue":true}}))
                .await
                .unwrap();
            sleep(Duration::from_secs(11)).await;
            server.close().await;
        };
        let (result, ()) =
            tokio::join!(watch_peer(&mut client, &c, &state, &mut timing), simulation);
        assert_eq!(result, Err(Error::Unavailable));
        assert_eq!(state.lock().unwrap().phase, "watching");
        assert!(timing.deadline().is_none());
    }
    #[tokio::test]
    async fn missing_launcher_fails_before_subscribing() {
        let (mut client, mut server) = tv::mock_pair().await;
        let c = crate::common::test_config(std::path::Path::new("/tmp"));
        let state = Arc::new(Mutex::new(State::new()));
        let mut timing = Timing::new(1);
        let simulation = async {
            server
                .next(Instant::now() + Duration::from_secs(1))
                .await
                .unwrap();
            server
                .send(json!({"id":"apps","payload":{"launchPoints":[]}}))
                .await
                .unwrap();
        };
        let (result, ()) =
            tokio::join!(watch_peer(&mut client, &c, &state, &mut timing), simulation);
        assert_eq!(
            result,
            Err(fault("Configured Home launcher is not installed."))
        );
    }
    #[test]
    fn duplicate_snapshots_and_double_home_escape() {
        let mut r = Redirect::default();
        let now = Instant::now();
        assert!(r.should_launch(LG_HOME, now));
        assert!(!r.should_launch(LG_HOME, now + Duration::from_secs(1)));
        assert!(!r.should_launch("native", now + Duration::from_secs(1)));
        assert!(!r.should_launch(LG_HOME, now + Duration::from_secs(2)));
        assert!(!r.should_launch("netflix", now + Duration::from_secs(4)));
        assert!(r.should_launch(LG_HOME, now + Duration::from_secs(5)));
    }
    #[test]
    fn heartbeat_distinguishes_standby_failure_and_stall() {
        let mut s = State::new();
        s.update("TV unavailable", None);
        assert_eq!(s.heartbeat(Instant::now()).0, "");
        assert_eq!(
            s.heartbeat(s.progress + Duration::from_secs(121)).0,
            "/fail"
        );
        let reason = "Configured Home launcher is not installed.";
        s.update("TV request failed", Some(reason));
        assert_eq!(s.heartbeat(Instant::now()), ("/fail", reason));
        s.update("TV unavailable", s.failure);
        assert_eq!(s.heartbeat(Instant::now()), ("/fail", reason));
        s.update("watching", None);
        assert_eq!(s.heartbeat(Instant::now()).0, "");
    }
    #[test]
    fn foreground_change_does_not_clear_launch_deadline() {
        let mut s = Subscription::new();
        let now = Instant::now();
        assert!(s
            .receive(&json!({"id":"fg","payload":{"appId":LG_HOME}}), now)
            .unwrap());
        assert!(!s
            .receive(&json!({"id":"fg","payload":{"appId":"native"}}), now)
            .unwrap());
        assert!(s.pending);
        assert!(s.deadline.is_some());
        s.receive(&json!({"id":"launch","payload":{"returnValue":true}}), now)
            .unwrap();
        assert!(!s.pending);
        assert!(s.deadline.is_none());
    }
    #[test]
    fn subscription_and_launch_failures_are_faults() {
        let mut s = Subscription::new();
        let now = Instant::now();
        assert!(s.receive(&json!({"id":"fg","type":"error"}), now).is_err());
        s.receive(&json!({"id":"fg","payload":{"appId":LG_HOME}}), now)
            .unwrap();
        assert!(s
            .receive(&json!({"id":"launch","payload":{"returnValue":false}}), now)
            .is_err());
    }
    #[tokio::test]
    async fn protocol_launches_native_and_checks_reply() {
        let (mut client, mut server) = tv::mock_pair().await;
        let c = crate::common::test_config(std::path::Path::new("/tmp"));
        let state = Arc::new(Mutex::new(State::new()));
        let mut timing = Timing::new(1);
        let simulation = async {
            server
                .next(Instant::now() + Duration::from_secs(1))
                .await
                .unwrap();
            server.send(json!({"id":"apps","payload":{"launchPoints":[{"id":"com.anant.tvhome.native"}]}})).await.unwrap();
            server
                .next(Instant::now() + Duration::from_secs(1))
                .await
                .unwrap();
            server
                .send(json!({"id":"fg","payload":{"appId":LG_HOME}}))
                .await
                .unwrap();
            let Event::Json(m) = server
                .next(Instant::now() + Duration::from_secs(1))
                .await
                .unwrap()
            else {
                panic!()
            };
            assert_eq!(m["payload"]["id"], "com.anant.tvhome.native");
            server
                .send(json!({"id":"launch","payload":{"returnValue":false}}))
                .await
                .unwrap();
        };
        let (result, ()) =
            tokio::join!(watch_peer(&mut client, &c, &state, &mut timing), simulation);
        assert!(matches!(result, Err(Error::Fault(_))));
    }
}
