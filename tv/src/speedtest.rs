//! Native, user-started transfers against Fast.com's discovered Netflix servers.
use anyhow::{bail, ensure, Context, Result};
use reqwest::{Client, Response, Url};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
    thread::JoinHandle,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tokio::{sync::watch, task::JoinSet};

const TRANSFER_TIME: Duration = Duration::from_secs(10);
const CONNECTIONS: usize = 4;
const MAX_BODY: usize = 2 * 1024 * 1024;
const UPLOAD_BUFFER: usize = 4 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Phase {
    Ready,
    Discovering,
    Latency,
    Download,
    Upload,
    Complete,
    Cancelled,
    Failed,
}
impl Phase {
    pub fn active(self) -> bool {
        matches!(
            self,
            Self::Discovering | Self::Latency | Self::Download | Self::Upload
        )
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct Measurement {
    pub download: f64,
    pub upload: f64,
    pub latency: f64,
    pub timestamp: u64,
    pub when: String,
}
impl Measurement {
    fn validate(&self) -> Result<()> {
        ensure!(
            [self.download, self.upload, self.latency]
                .iter()
                .all(|n| n.is_finite() && *n > 0.0),
            "Invalid speed test result"
        );
        ensure!(
            self.timestamp > 0 && self.when.len() <= 80 && !self.when.chars().any(char::is_control),
            "Invalid speed test date"
        );
        Ok(())
    }
    fn load(path: &Path) -> Result<Self> {
        let mut bytes = Vec::new();
        fs::File::open(path)?.take(4097).read_to_end(&mut bytes)?;
        ensure!(bytes.len() <= 4096, "Speed test result is too large");
        let result: Self = serde_json::from_slice(&bytes)?;
        result.validate()?;
        Ok(result)
    }
    fn save(&self, path: &Path) -> Result<()> {
        self.validate()?;
        fs::create_dir_all(path.parent().context("Missing result directory")?)?;
        let tmp = path.with_extension("json.tmp");
        let saved = (|| -> Result<()> {
            let mut file = fs::File::create(&tmp)?;
            file.write_all(&serde_json::to_vec(self)?)?;
            file.sync_all()?;
            fs::rename(&tmp, path)?;
            Ok(())
        })();
        if saved.is_err() {
            let _ = fs::remove_file(tmp);
        }
        saved
    }
}

#[derive(Clone)]
pub struct State {
    pub phase: Phase,
    pub download: Option<f64>,
    pub upload: Option<f64>,
    pub latency: Option<f64>,
    pub progress: f32,
    pub message: String,
    pub last: Option<Measurement>,
    pub preview: bool,
}
impl State {
    fn new(last: Option<Measurement>, preview: bool) -> Self {
        Self {
            phase: Phase::Ready,
            download: last.as_ref().map(|r| r.download),
            upload: last.as_ref().map(|r| r.upload),
            latency: last.as_ref().map(|r| r.latency),
            progress: 0.0,
            message: "Measure this device's internet connection.".into(),
            last,
            preview,
        }
    }
    pub fn button(&self) -> &'static str {
        if self.phase.active() {
            "Cancel test"
        } else if self.phase == Phase::Ready && self.last.is_none() {
            "Run speed test"
        } else {
            "Test again"
        }
    }
}

type Update = Arc<Mutex<Option<State>>>;
fn publish(slot: &Update, state: &State) {
    *slot.lock().unwrap() = Some(state.clone());
}

pub struct Monitor {
    pub state: State,
    path: PathBuf,
    slot: Update,
    worker: Option<JoinHandle<()>>,
    cancel: Option<watch::Sender<bool>>,
}
impl Monitor {
    pub fn new(path: PathBuf, preview: bool) -> Self {
        let last = match Measurement::load(&path) {
            Ok(last) => Some(last),
            Err(error) => {
                if path.exists() {
                    tracing::warn!(%error, "Could not read saved speed test");
                }
                None
            }
        };
        Self {
            state: State::new(last, preview),
            path,
            slot: Arc::new(Mutex::new(None)),
            worker: None,
            cancel: None,
        }
    }
    pub fn poll(&mut self) -> bool {
        let mut changed = false;
        if let Some(state) = self.slot.lock().unwrap().take() {
            if self.state.phase != Phase::Cancelled {
                self.state = state;
                changed = true;
            }
        }
        if self.worker.as_ref().is_some_and(|w| w.is_finished()) {
            if self.worker.take().unwrap().join().is_err() && self.state.phase.active() {
                self.state.phase = Phase::Failed;
                self.state.message = "The test stopped unexpectedly. Try again.".into();
                changed = true;
            }
            self.cancel = None;
        }
        changed
    }
    pub fn cancel(&mut self) {
        if self.state.phase.active() {
            if let Some(cancel) = &self.cancel {
                let _ = cancel.send(true);
            }
            self.state.phase = Phase::Cancelled;
            self.state.message = "Test cancelled. Your last result is unchanged.".into();
        }
    }
    pub fn toggle(&mut self) {
        self.poll();
        if self.state.phase.active() {
            self.cancel();
            return;
        }
        // The previous run must release its sockets before another can start.
        if self.worker.is_some() {
            return;
        }
        self.state = State::new(self.state.last.clone(), self.state.preview);
        self.state.phase = Phase::Discovering;
        self.state.download = None;
        self.state.upload = None;
        self.state.latency = None;
        self.state.message = "Finding Netflix test servers…".into();
        let (cancel, mut cancelled) = watch::channel(false);
        self.cancel = Some(cancel);
        let mut state = self.state.clone();
        let slot = self.slot.clone();
        let path = self.path.clone();
        self.worker = Some(std::thread::spawn(move || {
            let result = tokio::runtime::Builder::new_current_thread().enable_all().build()
                .map_err(anyhow::Error::from).and_then(|rt| rt.block_on(async {
                    tokio::select! {
                        biased;
                        _ = cancelled.changed() => Ok(None),
                        result = tokio::time::timeout(Duration::from_secs(65), measure(&mut state, &slot)) => result.context("The speed test timed out")?.map(Some),
                    }
                }));
            match result {
                Ok(Some(result)) => {
                    if *cancelled.borrow() {
                        return;
                    }
                    let saved = result.save(&path);
                    state.phase = Phase::Complete;
                    state.progress = 1.0;
                    state.message = if saved.is_ok() {
                        "Test complete"
                    } else {
                        "Test complete · could not save result"
                    }
                    .into();
                    tracing::info!(
                        download_mbps = result.download,
                        upload_mbps = result.upload,
                        latency_ms = result.latency,
                        preview = state.preview,
                        "Speed test complete"
                    );
                    if let Err(error) = saved {
                        tracing::warn!(%error, "Could not save speed test");
                    }
                    state.last = Some(result);
                }
                Ok(None) => {
                    return;
                }
                Err(error) => {
                    // Display only our context: underlying HTTP errors can contain signed URLs.
                    tracing::warn!(reason = %safe_error(&error), "Speed test failed");
                    state.phase = Phase::Failed;
                    state.message = failure_message(&error);
                }
            }
            publish(&slot, &state);
        }));
    }
}
impl Drop for Monitor {
    fn drop(&mut self) {
        self.cancel();
    }
}

fn safe_error(error: &anyhow::Error) -> String {
    let mut message = format!("{error:#}");
    for cause in error.chain() {
        if let Some(url) = cause.downcast_ref::<reqwest::Error>().and_then(|e| e.url()) {
            message = message.replace(url.as_str(), "[test server]");
        }
    }
    message
}

fn failure_message(error: &anyhow::Error) -> String {
    let reason = error.chain().find_map(|cause| {
        let error = cause.downcast_ref::<reqwest::Error>()?;
        if error.is_timeout() {
            Some("server timed out".to_owned())
        } else if let Some(status) = error.status() {
            Some(format!("server returned HTTP {}", status.as_u16()))
        } else if error.is_connect() {
            Some("could not connect to the server".to_owned())
        } else if error.is_body() || error.is_decode() {
            Some("connection interrupted".to_owned())
        } else {
            None
        }
    });
    match reason {
        Some(reason) => format!("{error}: {reason}. Try again."),
        None => format!("{error}. Try again."),
    }
}

fn client() -> Result<Client> {
    Client::builder()
        .https_only(true)
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(8))
        .user_agent("tvhome/0.1 (native speed test)")
        .build()
        .context("Could not prepare secure connections")
}

async fn bounded_body(mut response: Response, limit: usize) -> Result<Vec<u8>> {
    response
        .error_for_status_ref()
        .context("The test server rejected the request")?;
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .context("The test server stopped responding")?
    {
        ensure!(
            bytes.len().saturating_add(chunk.len()) <= limit,
            "The test server returned too much data"
        );
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}
fn script_path(html: &str) -> Result<&str> {
    for (offset, _) in html.match_indices("/app-") {
        let rest = &html[offset..];
        if let Some(end) = rest.find(".js") {
            let path = &rest[..end + 3];
            if end < 100
                && path[5..end]
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-')
                && end > 5
            {
                return Ok(path);
            }
        }
    }
    bail!("Fast.com changed its discovery page")
}
fn token(script: &str) -> Result<&str> {
    for (offset, _) in script.match_indices("token:") {
        let rest = script[offset + 6..].trim_start();
        if let Some(rest) = rest.strip_prefix('"') {
            if let Some(end) = rest.find('"') {
                let value = &rest[..end];
                if (16..=128).contains(&value.len())
                    && value.chars().all(|c| c.is_ascii_alphanumeric())
                {
                    return Ok(value);
                }
            }
        }
    }
    bail!("Fast.com changed its discovery script")
}
fn validate_target(url: &Url) -> Result<()> {
    ensure!(
        url.scheme() == "https"
            && url.port_or_known_default() == Some(443)
            && url.username().is_empty()
            && url.password().is_none()
            && url.fragment().is_none()
            && url
                .host_str()
                .is_some_and(|h| h.ends_with(".nflxvideo.net"))
            && matches!(
                url.path(),
                "/speedtest" | "/speedtest/" | "/speedtest/range/"
            ),
        "Fast.com returned an unsupported test server"
    );
    Ok(())
}
fn range_url(base: &Url, bytes: usize) -> Url {
    let mut url = base.clone();
    url.set_path(&format!("/speedtest/range/0-{}", bytes.saturating_sub(1)));
    url
}
async fn discover(client: &Client) -> Result<Vec<Url>> {
    let html = bounded_body(
        client
            .get("https://fast.com/")
            .send()
            .await
            .context("Could not reach Fast.com")?,
        MAX_BODY,
    )
    .await?;
    let html = std::str::from_utf8(&html).context("Fast.com returned an unreadable page")?;
    let js_url = format!("https://fast.com{}", script_path(html)?);
    let js = bounded_body(
        client
            .get(js_url)
            .send()
            .await
            .context("Could not load Fast.com's discovery script")?,
        MAX_BODY,
    )
    .await?;
    let js = std::str::from_utf8(&js).context("Fast.com returned an unreadable script")?;
    let token = token(js)?;
    let reply = client
        .get("https://api.fast.com/netflix/speedtest/v2")
        .query(&[("https", "true"), ("token", token), ("urlCount", "4")])
        .send()
        .await
        .context("Could not find Netflix test servers")?;
    #[derive(Deserialize)]
    struct Target {
        url: String,
    }
    #[derive(Deserialize)]
    struct Discovery {
        targets: Vec<Target>,
    }
    let data: Discovery = serde_json::from_slice(&bounded_body(reply, 65536).await?)
        .context("Fast.com returned an unsupported server list")?;
    ensure!(
        !data.targets.is_empty() && data.targets.len() <= 16,
        "Fast.com returned no usable test servers"
    );
    data.targets
        .into_iter()
        .take(CONNECTIONS)
        .map(|target| {
            let url =
                Url::parse(&target.url).context("Fast.com returned an invalid server address")?;
            validate_target(&url)?;
            Ok(url)
        })
        .collect()
}
fn mbps(bytes: u64, elapsed: Duration) -> f64 {
    bytes as f64 * 8.0 / elapsed.as_secs_f64().max(0.001) / 1_000_000.0
}

async fn measure(state: &mut State, slot: &Update) -> Result<Measurement> {
    let client = client()?;
    let targets = discover(&client).await?;
    state.phase = Phase::Latency;
    state.message = "Measuring latency…".into();
    state.progress = 0.08;
    publish(slot, state);
    let latency = measure_latency(&client, &targets[0]).await?;
    state.latency = Some(latency);
    let down = transfer(&client, &targets, false, TRANSFER_TIME, state, slot)
        .await
        .context("Download test failed")?;
    state.download = Some(down);
    let up = transfer(&client, &targets, true, TRANSFER_TIME, state, slot)
        .await
        .context("Upload test failed")?;
    state.upload = Some(up);
    Ok(Measurement {
        download: down,
        upload: up,
        latency,
        timestamp: SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs(),
        when: crate::platform::clock(c"%e %b, %l:%M %p").trim().to_owned(),
    })
}

async fn measure_latency(client: &Client, target: &Url) -> Result<f64> {
    let mut latency = Vec::new();
    // Warm TLS first; measure the median of three small HTTP round trips.
    for i in 0..4 {
        let start = Instant::now();
        // Netflix rejects an empty POST without an explicit Content-Length.
        let response = client
            .post(range_url(target, 1))
            .header("Content-Length", "0")
            .body("")
            .send()
            .await
            .context("Could not measure latency")?;
        bounded_body(response, 4096)
            .await
            .context("Could not measure latency")?;
        if i > 0 {
            latency.push(start.elapsed().as_secs_f64() * 1000.0);
        }
    }
    latency.sort_by(f64::total_cmp);
    Ok(latency[1])
}

async fn transfer(
    client: &Client,
    targets: &[Url],
    upload: bool,
    duration: Duration,
    state: &mut State,
    slot: &Update,
) -> Result<f64> {
    state.phase = if upload {
        Phase::Upload
    } else {
        Phase::Download
    };
    state.message = if upload {
        "Measuring upload…"
    } else {
        "Measuring download…"
    }
    .into();
    publish(slot, state);
    let bytes = Arc::new(AtomicU64::new(0));
    let start = Instant::now();
    let deadline = start + duration;
    // One shared, bounded buffer; downloaded chunks are discarded immediately.
    let payload = bytes::Bytes::from(vec![0x5a; if upload { UPLOAD_BUFFER } else { 0 }]);
    let mut jobs = JoinSet::new();
    for i in 0..CONNECTIONS {
        let client = client.clone();
        let target = targets[i % targets.len()].clone();
        let bytes = bytes.clone();
        let payload = payload.clone();
        jobs.spawn(async move {
            let mut size = 256 * 1024;
            while Instant::now() < deadline {
                let began = Instant::now();
                if upload {
                    let response = client
                        .post(range_url(&target, size))
                        .header("Content-Type", "application/octet-stream")
                        .body(payload.slice(..size))
                        .send()
                        .await
                        .context("Upload connection failed")?;
                    bounded_body(response, 65536)
                        .await
                        .context("Upload was not acknowledged")?;
                    // Count only complete uploads acknowledged by a successful response.
                    bytes.fetch_add(size as u64, Ordering::Relaxed);
                    size = ((size as f64 * 0.25 / began.elapsed().as_secs_f64().max(0.001))
                        as usize)
                        .clamp(16 * 1024, UPLOAD_BUFFER);
                } else {
                    let transfer = async {
                        let mut response = client
                            .get(range_url(&target, 16 * 1024 * 1024))
                            // Let the measurement deadline end an unfinished response normally.
                            // The client's shorter total timeout can otherwise abort a healthy stream.
                            .timeout(duration + Duration::from_secs(1))
                            .send()
                            .await
                            .context("Download connection failed")?;
                        response
                            .error_for_status_ref()
                            .context("Download was rejected")?;
                        let mut received = 0;
                        while let Some(chunk) =
                            response.chunk().await.context("Download interrupted")?
                        {
                            received += chunk.len();
                            bytes.fetch_add(chunk.len() as u64, Ordering::Relaxed);
                        }
                        ensure!(received > 0, "Empty download response");
                        Ok::<_, anyhow::Error>(())
                    };
                    match tokio::time::timeout_at(deadline.into(), transfer).await {
                        Ok(result) => result?,
                        Err(_) => break,
                    }
                }
            }
            Ok::<_, anyhow::Error>(())
        });
    }
    let mut ticks = tokio::time::interval(Duration::from_millis(200));
    while !jobs.is_empty() {
        tokio::select! {
            result = jobs.join_next() => { result.context("Test worker stopped")?.context("Test worker failed")??; }
            _ = ticks.tick() => {
                let rate = mbps(bytes.load(Ordering::Relaxed), start.elapsed());
                if upload { state.upload = Some(rate); } else { state.download = Some(rate); }
                state.progress = (if upload { 0.55 } else { 0.1 }) + 0.45 * (start.elapsed().as_secs_f32() / duration.as_secs_f32()).min(1.0);
                publish(slot, state);
            }
        }
    }
    let count = bytes.load(Ordering::Relaxed);
    ensure!(count > 0, "No data transferred");
    Ok(mbps(count, start.elapsed()))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn discovery_parsing_is_bounded_and_target_is_https_netflix() {
        assert_eq!(
            script_path(r#"<script src="/app-ab12.js"></script>"#).unwrap(),
            "/app-ab12.js"
        );
        assert!(script_path(r#"/app-../../x.js"#).is_err());
        assert_eq!(
            token(r#"token: "0123456789abcdef""#).unwrap(),
            "0123456789abcdef"
        );
        assert!(token(r#"token:"x""#).is_err());
        let good = Url::parse("https://a.nflxvideo.net/speedtest/range/?signed=secret").unwrap();
        validate_target(&good).unwrap();
        let current = Url::parse("https://a.nflxvideo.net/speedtest?signed=secret").unwrap();
        validate_target(&current).unwrap();
        assert_eq!(range_url(&current, 1024).path(), "/speedtest/range/0-1023");
        assert_eq!(range_url(&current, 1024).query(), current.query());
        assert_eq!(range_url(&good, 1024).path(), "/speedtest/range/0-1023");
        assert_eq!(range_url(&good, 0).query(), good.query());
        for url in [
            "http://a.nflxvideo.net/speedtest/range/",
            "https://a.nflxvideo.net.evil.test/speedtest/range/",
            "https://user@a.nflxvideo.net/speedtest/range/",
            "https://a.nflxvideo.net:444/speedtest/range/",
            "https://a.nflxvideo.net/elsewhere",
        ] {
            assert!(validate_target(&Url::parse(url).unwrap()).is_err());
        }
        assert_eq!(mbps(25_000_000, Duration::from_secs(2)), 100.0);
    }
    #[test]
    fn completed_result_survives_restart_and_bad_write_preserves_it() {
        let root = std::env::temp_dir().join(format!("tvhome-speedtest-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let path = root.join("result.json");
        let result = Measurement {
            download: 210.0,
            upload: 180.0,
            latency: 8.0,
            timestamp: 1234,
            when: "20 Oct, 8:00 PM".into(),
        };
        result.save(&path).unwrap();
        let monitor = Monitor::new(path.clone(), true);
        assert_eq!(monitor.state.last, Some(result.clone()));
        assert_eq!(monitor.state.phase, Phase::Ready);
        assert!(monitor.worker.is_none());
        let mut invalid = result.clone();
        invalid.download = f64::NAN;
        assert!(invalid.save(&path).is_err());
        assert_eq!(Measurement::load(&path).unwrap(), result);
        fs::write(&path, b"broken").unwrap();
        assert!(Monitor::new(path, true).state.last.is_none());
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn cancelling_ignores_late_worker_updates() {
        let mut monitor = Monitor::new(PathBuf::from("/nonexistent/speedtest.json"), true);
        let (tx, rx) = watch::channel(false);
        monitor.cancel = Some(tx);
        monitor.state.phase = Phase::Download;
        monitor.cancel();
        assert!(*rx.borrow());
        let mut late = monitor.state.clone();
        late.phase = Phase::Complete;
        publish(&monitor.slot, &late);
        assert!(!monitor.poll());
        assert_eq!(monitor.state.phase, Phase::Cancelled);
    }
    async fn fixture(status: u16) -> (Url, tokio::task::JoinHandle<()>, Arc<AtomicU64>) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = Url::parse(&format!(
            "http://{}/speedtest/range/",
            listener.local_addr().unwrap()
        ))
        .unwrap();
        let received = Arc::new(AtomicU64::new(0));
        let total = received.clone();
        let task = tokio::spawn(async move {
            let mut connections = JoinSet::new();
            loop {
                tokio::select! {
                    accepted = listener.accept() => {
                        let (mut socket, _) = accepted.unwrap();
                        let total = total.clone();
                        connections.spawn(async move {
                            let mut header = Vec::new();
                            let mut byte = [0];
                            while !header.ends_with(b"\r\n\r\n") {
                                if socket.read_exact(&mut byte).await.is_err() || header.len() >= 8192 { return; }
                                header.push(byte[0]);
                            }
                            let header = String::from_utf8_lossy(&header);
                            let size: usize = header.lines().find_map(|line| line.to_ascii_lowercase().strip_prefix("content-length:").map(|s| s.trim().parse().unwrap())).unwrap_or(0);
                            let mut body = vec![0; size];
                            if socket.read_exact(&mut body).await.is_err() { return; }
                            total.fetch_add(size as u64, Ordering::Relaxed);
                            // Delay replies to make progress and cancellation observable.
                            tokio::time::sleep(Duration::from_millis(15)).await;
                            let response_size = if header.starts_with("GET") { 8192 } else { 0 };
                            let status = if header.starts_with("POST") && !header.to_ascii_lowercase().contains("content-length:") { 400 } else { status };
                            let reply = format!("HTTP/1.1 {status} Test\r\nContent-Length: {response_size}\r\nConnection: close\r\n\r\n");
                            if socket.write_all(reply.as_bytes()).await.is_ok() {
                                let _ = socket.write_all(&vec![0; response_size]).await;
                            }
                        });
                    }
                    _ = connections.join_next(), if !connections.is_empty() => (),
                }
            }
        });
        (url, task, received)
    }

    #[tokio::test(flavor = "current_thread")]
    async fn real_http_transfers_measure_bytes_and_reject_failed_uploads() {
        let client = Client::builder()
            .timeout(Duration::from_secs(1))
            .build()
            .unwrap();
        let (url, server, received) = fixture(200).await;
        let slot = Arc::new(Mutex::new(None));
        let mut state = State::new(None, true);
        assert!(measure_latency(&client, &url).await.unwrap() > 0.0);
        for upload in [false, true] {
            let rate = transfer(
                &client,
                std::slice::from_ref(&url),
                upload,
                Duration::from_millis(120),
                &mut state,
                &slot,
            )
            .await
            .unwrap();
            assert!(rate.is_finite() && rate > 0.0);
        }
        assert!(received.load(Ordering::Relaxed) > 0);
        server.abort();
        let (url, server, _) = fixture(503).await;
        let mut signed = url.clone();
        signed.set_query(Some("token=private-test-value"));
        let error = client
            .get(signed)
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap_err();
        let error: anyhow::Error = error.into();
        let message = safe_error(&error);
        assert!(!message.contains("private-test-value"));
        assert!(message.contains("503"));
        let message = failure_message(&error.context("Upload test failed"));
        assert_eq!(
            message,
            "Upload test failed: server returned HTTP 503. Try again."
        );
        let mut state = State::new(None, true);
        assert!(transfer(
            &client,
            &[url],
            true,
            Duration::from_millis(120),
            &mut state,
            &slot
        )
        .await
        .is_err());
        assert!(state.upload.unwrap_or(0.0) == 0.0);
        assert!(state.last.is_none());
        server.abort();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn dropping_a_running_test_stops_transfers_and_body_limits_are_enforced() {
        let client = Client::builder()
            .timeout(Duration::from_secs(1))
            .build()
            .unwrap();
        let (url, server, received) = fixture(200).await;
        let slot = Arc::new(Mutex::new(None));
        let mut state = State::new(None, true);
        let targets = [url.clone()];
        let test = transfer(
            &client,
            &targets,
            true,
            Duration::from_secs(10),
            &mut state,
            &slot,
        );
        assert!(tokio::time::timeout(Duration::from_millis(80), test)
            .await
            .is_err());
        tokio::time::sleep(Duration::from_millis(30)).await;
        let at_cancel = received.load(Ordering::Relaxed);
        assert!(at_cancel > 0);
        tokio::time::sleep(Duration::from_millis(80)).await;
        assert_eq!(received.load(Ordering::Relaxed), at_cancel);
        let response = client.get(url).send().await.unwrap();
        assert!(bounded_body(response, 32).await.is_err());
        server.abort();
    }
    async fn slow_download_server() -> (Url, tokio::task::JoinHandle<()>) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = Url::parse(&format!(
            "http://{}/speedtest/range/",
            listener.local_addr().unwrap()
        ))
        .unwrap();
        let server = tokio::spawn(async move {
            let mut connections = JoinSet::new();
            loop {
                tokio::select! {
                    accepted = listener.accept() => {
                        let (mut stream, _) = accepted.unwrap();
                        connections.spawn(async move {
                            let mut header = Vec::new();
                            let mut byte = [0];
                            while !header.ends_with(b"\r\n\r\n") {
                                if header.len() >= 8192 || stream.read_exact(&mut byte).await.is_err() { return; }
                                header.push(byte[0]);
                            }
                            if stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 16777216\r\nConnection: close\r\n\r\n").await.is_err() { return; }
                            // Keep making progress, but never finish the response within the test.
                            loop {
                                if stream.write_all(&[0; 4096]).await.is_err() { break; }
                                tokio::time::sleep(Duration::from_millis(20)).await;
                            }
                        });
                    }
                    _ = connections.join_next(), if !connections.is_empty() => (),
                }
            }
        });
        (url, server)
    }

    #[tokio::test(flavor = "current_thread")]
    async fn progressing_download_outlives_default_request_timeout() {
        let client = Client::builder()
            .timeout(Duration::from_millis(200))
            .build()
            .unwrap();
        let (url, server) = slow_download_server().await;
        let response = client.get(url.clone()).send().await.unwrap();
        let error = bounded_body(response, MAX_BODY)
            .await
            .unwrap_err()
            .context("Download test failed");
        assert_eq!(
            failure_message(&error),
            "Download test failed: server timed out. Try again."
        );
        let mut state = State::new(None, true);
        let slot = Arc::new(Mutex::new(None));
        let start = Instant::now();
        let duration = Duration::from_millis(500);
        let result = transfer(&client, &[url], false, duration, &mut state, &slot).await;
        server.abort();
        assert!(
            result.is_ok(),
            "A progressing download must run until the measurement deadline: {result:?}"
        );
        assert!(start.elapsed() >= duration);
        assert!(start.elapsed() < Duration::from_secs(2));
        assert!(result.unwrap() > 0.0);
    }
}
