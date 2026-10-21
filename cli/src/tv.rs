use crate::common::{fault, Config, Error, Result};
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use std::time::Duration;
use tokio::{
    io::{AsyncRead, AsyncWrite},
    net::TcpStream,
    time::{timeout, Instant},
};
use tokio_tungstenite::{
    connect_async_tls_with_config,
    tungstenite::{protocol::WebSocketConfig, Message},
    Connector, MaybeTlsStream, WebSocketStream,
};

// Registration permissions used by LG's remote-control API.
const PERMISSIONS: &[&str] = &[
    "LAUNCH",
    "LAUNCH_WEBAPP",
    "APP_TO_APP",
    "CLOSE",
    "TEST_OPEN",
    "TEST_PROTECTED",
    "CONTROL_AUDIO",
    "CONTROL_DISPLAY",
    "CONTROL_INPUT_JOYSTICK",
    "CONTROL_INPUT_MEDIA_RECORDING",
    "CONTROL_INPUT_MEDIA_PLAYBACK",
    "CONTROL_INPUT_TV",
    "CONTROL_POWER",
    "CONTROL_TV_SCREEN",
    "READ_APP_STATUS",
    "READ_CURRENT_CHANNEL",
    "READ_INPUT_DEVICE_LIST",
    "READ_NETWORK_STATE",
    "READ_RUNNING_APPS",
    "READ_TV_CHANNEL_LIST",
    "WRITE_NOTIFICATION_TOAST",
    "READ_POWER_STATE",
    "READ_COUNTRY_INFO",
    "CONTROL_INPUT_TEXT",
    "CONTROL_MOUSE_AND_KEYBOARD",
    "READ_INSTALLED_APPS",
    "READ_SETTINGS",
    "READ_STORAGE_DEVICE_LIST",
];
pub struct Peer<S> {
    pub socket: WebSocketStream<S>,
}
pub enum Event {
    Json(Value),
    Pong,
}
impl<S: AsyncRead + AsyncWrite + Unpin> Peer<S> {
    pub async fn send(&mut self, value: Value) -> Result<()> {
        timeout(
            Duration::from_secs(10),
            self.socket.send(Message::Text(value.to_string().into())),
        )
        .await
        .map_err(|_| fault("TV write timed out."))?
        .map_err(|_| Error::Unavailable)
    }
    pub async fn next(&mut self, deadline: Instant) -> Result<Event> {
        loop {
            let message = tokio::time::timeout_at(deadline, self.socket.next())
                .await
                .map_err(|_| fault("Connected TV stopped responding."))?
                .ok_or(Error::Unavailable)?
                .map_err(|_| Error::Unavailable)?;
            match message {
                Message::Text(text) => {
                    let value: Value = serde_json::from_str(&text)
                        .map_err(|_| fault("TV returned invalid JSON."))?;
                    if !value.is_object() {
                        return Err(fault("TV returned an invalid message."));
                    }
                    return Ok(Event::Json(value));
                }
                Message::Pong(_) => return Ok(Event::Pong),
                Message::Ping(bytes) => {
                    timeout(
                        Duration::from_secs(5),
                        self.socket.send(Message::Pong(bytes)),
                    )
                    .await
                    .map_err(|_| fault("TV ping reply timed out."))?
                    .map_err(|_| Error::Unavailable)?;
                }
                Message::Close(_) => return Err(Error::Unavailable),
                _ => return Err(fault("TV returned an unexpected frame.")),
            }
        }
    }
    pub async fn request(&mut self, uri: &str, id: &str) -> Result<Value> {
        self.send(json!({"type":"request","id":id,"uri":uri,"payload":{}}))
            .await?;
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Event::Json(value) = self.next(deadline).await? {
                if value["id"] == id {
                    return Ok(payload(&value)?.clone());
                }
            }
        }
    }
    pub async fn ping(&mut self) -> Result<()> {
        timeout(
            Duration::from_secs(5),
            self.socket.send(Message::Ping(Vec::new().into())),
        )
        .await
        .map_err(|_| fault("TV ping timed out."))?
        .map_err(|_| Error::Unavailable)
    }
    pub async fn close(&mut self) {
        let _ = timeout(Duration::from_secs(3), self.socket.close(None)).await;
    }
}
pub fn payload(message: &Value) -> Result<&Value> {
    let p = &message["payload"];
    if message["type"] == "error" || !p.is_object() || p["returnValue"] == false {
        return Err(fault("TV rejected a request."));
    }
    Ok(p)
}
pub async fn register<S: AsyncRead + AsyncWrite + Unpin>(
    peer: &mut Peer<S>,
    config: &Config,
    pair: bool,
) -> Result<()> {
    let mut data = json!({"forcePairing":false,"pairingType":"PROMPT","manifest":{"manifestVersion":1,"appVersion":"1.1","permissions":PERMISSIONS}});
    if let Some(key) = config.key() {
        data["client-key"] = key.into();
    } else if !pair {
        return Err(fault(
            "No pairing key. Run tvhome --tv NAME pair or supply TV_CLIENT_KEY/TV_CLIENT_KEY_FILE.",
        ));
    }
    if pair {
        crate::log("Accept the TV pairing prompt if one appears.");
    }
    peer.send(json!({"type":"register","id":"register_0","payload":data}))
        .await?;
    let deadline = Instant::now() + Duration::from_secs(if pair { 60 } else { 15 });
    loop {
        if let Event::Json(message) = peer.next(deadline).await? {
            let data = payload(&message)?;
            if message["type"] == "registered" {
                if pair {
                    let key = data["client-key"]
                        .as_str()
                        .filter(|k| !k.is_empty())
                        .ok_or(fault("TV did not supply a pairing key."))?;
                    config.save_key(key)?;
                }
                return Ok(());
            }
            if !pair {
                return Err(fault(
                    "TV requires pairing again; run tvhome --tv NAME pair interactively.",
                ));
            }
        }
    }
}
pub async fn connect(config: &Config, pair: bool) -> Result<Peer<MaybeTlsStream<TcpStream>>> {
    if !pair && config.key().is_none() {
        return Err(fault("No pairing key. Run tvhome --tv NAME pair first."));
    }
    let url = format!("wss://{}:3001", config.host()?);
    // The LG local endpoint uses its own certificate; this exception applies ONLY to this TV socket.
    let tls = native_tls::TlsConnector::builder()
        .danger_accept_invalid_certs(true)
        .danger_accept_invalid_hostnames(true)
        .build()
        .map_err(|_| fault("Cannot initialize the TV TLS connection."))?;
    let ws_config = WebSocketConfig::default().max_message_size(Some(2 * 1024 * 1024));
    let (socket, _) = timeout(
        Duration::from_secs(10),
        connect_async_tls_with_config(url, Some(ws_config), false, Some(Connector::NativeTls(tls))),
    )
    .await
    .map_err(|_| Error::Unavailable)?
    .map_err(|_| Error::Unavailable)?;
    let mut peer = Peer { socket };
    if let Err(error) = register(&mut peer, config, pair).await {
        peer.close().await;
        return Err(error);
    }
    Ok(peer)
}

#[cfg(test)]
pub async fn mock_pair() -> (Peer<tokio::io::DuplexStream>, Peer<tokio::io::DuplexStream>) {
    use tokio_tungstenite::tungstenite::protocol::Role;
    let (a, b) = tokio::io::duplex(65536);
    (
        Peer {
            socket: WebSocketStream::from_raw_socket(a, Role::Client, None).await,
        },
        Peer {
            socket: WebSocketStream::from_raw_socket(b, Role::Server, None).await,
        },
    )
}
#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn pairing_saves_key_without_launching() {
        let dir = tempfile::tempdir().unwrap();
        let mut c = crate::common::test_config(dir.path());
        c.values.remove("TV_CLIENT_KEY");
        let (mut client, mut server) = mock_pair().await;
        let server_task = async {
            let Event::Json(m) = server
                .next(Instant::now() + Duration::from_secs(1))
                .await
                .unwrap()
            else {
                panic!()
            };
            assert_eq!(m["type"], "register");
            assert!(m["payload"].get("client-key").is_none());
            server
                .send(json!({"type":"registered","payload":{"client-key":"saved-key"}}))
                .await
                .unwrap();
        };
        let (result, ()) = tokio::join!(register(&mut client, &c, true), server_task);
        result.unwrap();
        assert_eq!(c.key().unwrap(), "saved-key");
    }
    #[test]
    fn errors_and_failed_payloads_are_not_successes() {
        for m in [
            json!({"type":"error","error":"secret"}),
            json!({"payload":{"returnValue":false}}),
            json!({}),
        ] {
            assert!(!payload(&m).unwrap_err().to_string().contains("secret"));
        }
    }
}
