use crate::common::{fault, matches, private_write, Config, Result};
use base64::{engine::general_purpose::STANDARD_NO_PAD, Engine};
use serde_json::Value;
use sha2::{Digest, Sha256};
use ssh2::{CheckResult, ExtendedData, KnownHostFileKind, Session};
use std::{
    io::{Read, Write},
    net::{TcpStream, ToSocketAddrs},
    path::PathBuf,
    time::{Duration, Instant},
};

pub const TOKEN_COMMAND: &str = "cat /var/luna/preferences/devmode_enabled";
const MAX_REPLY: usize = 4 * 1024 * 1024;

pub fn key_path(config: &Config) -> PathBuf {
    config
        .get("TV_SSH_KEY")
        .map(|p| config.path(p))
        .unwrap_or_else(|| config.profile_root().join(&config.tv).join("ssh-key"))
}
pub fn known_hosts_path(config: &Config) -> PathBuf {
    config
        .get("TV_SSH_KNOWN_HOSTS")
        .map(|p| config.path(p))
        .unwrap_or_else(|| config.profile_root().join(&config.tv).join("known_hosts"))
}
pub fn endpoint(config: &Config) -> Result<(&str, &str, u16)> {
    let host = config.host()?;
    let user = config.get("TV_SSH_USER").unwrap_or("prisoner");
    if !matches(r"^[A-Za-z_][A-Za-z0-9_-]*$", user) {
        return Err(fault("Invalid TV_SSH_USER."));
    }
    let port = config
        .get("TV_SSH_PORT")
        .unwrap_or("9922")
        .parse::<u16>()
        .ok()
        .filter(|p| *p > 0)
        .ok_or(fault("Invalid TV_SSH_PORT."))?;
    Ok((host, user, port))
}
pub fn validate(config: &Config) -> Result<()> {
    endpoint(config)?;
    if !key_path(config).is_file() {
        return Err(fault("TV SSH key is missing. Run tvhome --tv NAME setup."));
    }
    Ok(())
}
pub fn quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

pub struct TvSsh(Session);
impl TvSsh {
    pub fn connect(config: &Config, interactive: bool) -> Result<Self> {
        validate(config)?;
        let (host, user, port) = endpoint(config)?;
        let addresses = (host, port)
            .to_socket_addrs()
            .map_err(|_| fault("Cannot resolve the TV address."))?;
        let mut tcp = None;
        for address in addresses.take(4) {
            if let Ok(stream) = TcpStream::connect_timeout(&address, Duration::from_secs(5)) {
                tcp = Some(stream);
                break;
            }
        }
        let tcp = tcp.ok_or(fault(
            "Cannot reach Developer Mode SSH. Check the TV address and Developer Mode status.",
        ))?;
        tcp.set_read_timeout(Some(Duration::from_secs(15))).ok();
        tcp.set_write_timeout(Some(Duration::from_secs(15))).ok();
        let mut session = Session::new().map_err(|_| fault("Cannot initialize SSH."))?;
        session.set_tcp_stream(tcp);
        session.set_timeout(15_000);
        session
            .handshake()
            .map_err(|_| fault("Developer Mode SSH handshake failed."))?;
        let (key, kind) = session
            .host_key()
            .ok_or(fault("TV supplied no SSH host key."))?;
        let path = known_hosts_path(config);
        let mut known = session
            .known_hosts()
            .map_err(|_| fault("Cannot initialize SSH host verification."))?;
        if path.exists() {
            known
                .read_file(&path, KnownHostFileKind::OpenSSH)
                .map_err(|_| fault("Cannot read the TV's known-hosts file."))?;
        }
        match known.check_port(host, port, key) {
            CheckResult::Match => (),
            CheckResult::NotFound if interactive => {
                let fingerprint = STANDARD_NO_PAD.encode(Sha256::digest(key));
                println!("TV at {host}:{port} has SSH fingerprint SHA256:{fingerprint}");
                if !crate::setup::confirm("Confirm this is your TV and trust this key? [y/N] ")? {
                    return Err(fault("TV SSH identity was not approved."));
                }
                known.add(&format!("[{host}]:{port}"), key, "tvhome", kind.into())
                    .map_err(|_| fault("Cannot record the TV's SSH identity."))?;
                let mut lines = String::new();
                for entry in known.hosts().map_err(|_| fault("Cannot read SSH identities."))? {
                    lines.push_str(&known.write_string(&entry, KnownHostFileKind::OpenSSH)
                        .map_err(|_| fault("Cannot encode SSH identity."))?);
                }
                private_write(&path, lines.as_bytes())?;
            }
            CheckResult::NotFound => return Err(fault("TV SSH identity is not trusted. Run setup interactively.")),
            CheckResult::Mismatch => return Err(fault("TV SSH identity changed. Verify the TV and its address before replacing its known-hosts entry.")),
            CheckResult::Failure => return Err(fault("TV SSH identity verification failed.")),
        }
        session.userauth_pubkey_file(user, None, &key_path(config), None)
            .map_err(|_| fault("TV SSH authentication failed. Run setup --refresh-key with Key Server enabled."))?;
        if !session.authenticated() {
            return Err(fault("TV SSH authentication failed."));
        }
        Ok(Self(session))
    }
    pub fn command(&self, command: &str) -> Result<Vec<u8>> {
        let mut channel = self
            .0
            .channel_session()
            .map_err(|_| fault("Cannot open TV SSH channel."))?;
        channel
            .handle_extended_data(ExtendedData::Ignore)
            .map_err(|_| fault("Cannot configure SSH channel."))?;
        channel
            .exec(command)
            .map_err(|_| fault("Cannot start TV command."))?;
        channel
            .send_eof()
            .map_err(|_| fault("Cannot finish TV command input."))?;
        let mut bytes = Vec::new();
        (&mut channel)
            .take((MAX_REPLY + 1) as u64)
            .read_to_end(&mut bytes)
            .map_err(|_| fault("TV command timed out or failed."))?;
        if bytes.len() > MAX_REPLY {
            return Err(fault("TV reply exceeded its size limit."));
        }
        channel
            .wait_close()
            .map_err(|_| fault("TV command did not close."))?;
        if channel
            .exit_status()
            .map_err(|_| fault("Missing TV command status."))?
            != 0
        {
            return Err(fault("TV command failed."));
        }
        Ok(bytes)
    }
    pub fn upload(&self, path: &str, bytes: &[u8]) -> Result<()> {
        // Streaming over SSH also works on firmware without an SFTP subsystem.
        let mut channel = self
            .0
            .channel_session()
            .map_err(|_| fault("Cannot open upload channel."))?;
        channel
            .handle_extended_data(ExtendedData::Ignore)
            .map_err(|_| fault("Cannot configure upload channel."))?;
        channel
            .exec(&format!("umask 077; cat > {}", quote(path)))
            .map_err(|_| fault("Cannot start package upload."))?;
        channel
            .write_all(bytes)
            .map_err(|_| fault("Package upload failed."))?;
        channel
            .send_eof()
            .map_err(|_| fault("Cannot finish package upload."))?;
        let mut reply = Vec::new();
        (&mut channel)
            .take(4096)
            .read_to_end(&mut reply)
            .map_err(|_| fault("Package upload did not finish."))?;
        channel
            .wait_close()
            .map_err(|_| fault("Package upload did not close."))?;
        if channel
            .exit_status()
            .map_err(|_| fault("Missing upload status."))?
            != 0
        {
            return Err(fault("Package upload failed."));
        }
        Ok(())
    }
    pub fn luna(&self, uri: &str, parameters: &Value, terminal: Option<&str>) -> Result<Value> {
        let mode = if terminal.is_some() { "-i" } else { "-n 1" };
        let command = format!(
            "/usr/bin/luna-send-pub {mode} {} {}",
            quote(uri),
            quote(&parameters.to_string())
        );
        let mut channel = self
            .0
            .channel_session()
            .map_err(|_| fault("Cannot open TV service channel."))?;
        channel
            .handle_extended_data(ExtendedData::Ignore)
            .map_err(|_| fault("Cannot configure TV service channel."))?;
        channel
            .exec(&command)
            .map_err(|_| fault("Cannot call TV service."))?;
        let deadline = Instant::now() + Duration::from_secs(120);
        let mut pending = Vec::new();
        let mut total = 0;
        let mut chunk = [0u8; 4096];
        loop {
            if Instant::now() >= deadline {
                return Err(fault("TV service timed out."));
            }
            let count = channel
                .read(&mut chunk)
                .map_err(|_| fault("TV service stopped responding."))?;
            if count == 0 {
                return Err(fault("TV service closed before confirming success."));
            }
            pending.extend_from_slice(&chunk[..count]);
            total += count;
            if total > MAX_REPLY {
                return Err(fault("TV service reply exceeded its size limit."));
            }
            let mut stream = serde_json::Deserializer::from_slice(&pending).into_iter::<Value>();
            let mut consumed = 0;
            while let Some(message) = stream.next() {
                let value = match message {
                    Ok(v) => v,
                    Err(e) if e.is_eof() => break,
                    Err(_) => return Err(fault("TV service returned invalid JSON.")),
                };
                consumed = stream.byte_offset();
                if response_complete(&value, terminal)? {
                    let _ = channel.send_eof();
                    let _ = channel.close();
                    return Ok(value);
                }
            }
            pending.drain(..consumed);
        }
    }
}
pub fn response_complete(value: &Value, terminal: Option<&str>) -> Result<bool> {
    if !value.is_object() || value["returnValue"] == false {
        return Err(fault("TV rejected the service request."));
    }
    let Some(expected) = terminal else {
        if value["returnValue"] == true {
            return Ok(true);
        }
        return Err(fault("TV service did not confirm success."));
    };
    let state = value["details"]["state"]
        .as_str()
        .or(value["status"].as_str())
        .unwrap_or("")
        .to_ascii_uppercase();
    if state.contains("FAIL") || state.contains("ERROR") {
        return Err(fault("TV could not complete the app operation."));
    }
    Ok(state == expected.to_ascii_uppercase() || state.starts_with("SUCCESS"))
}
pub async fn read_token(config: &Config) -> Result<String> {
    let config = config.clone();
    tokio::task::spawn_blocking(move || {
        let bytes = TvSsh::connect(&config, false)?.command(TOKEN_COMMAND)?;
        let text = std::str::from_utf8(&bytes)
            .map_err(|_| fault("Invalid Developer Mode token encoding."))?;
        crate::renew::valid_token(text.trim())
    })
    .await
    .map_err(|_| fault("TV SSH task failed."))?
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    #[ignore = "run through tests/ssh-fixture.cjs in the native builder"]
    fn real_ssh_fixture() {
        let dir = std::path::PathBuf::from(
            std::env::var_os("TVHOME_SSH_FIXTURE").expect("SSH fixture directory"),
        );
        let profile = tempfile::tempdir().unwrap();
        let mut config = crate::common::test_config(profile.path());
        config.values.insert("TV_HOST".into(), "127.0.0.1".into());
        config.values.insert(
            "TV_SSH_PORT".into(),
            std::fs::read_to_string(dir.join("port")).unwrap(),
        );
        config.values.insert(
            "TV_SSH_KEY".into(),
            dir.join("key").to_str().unwrap().into(),
        );
        config.values.insert(
            "TV_SSH_KNOWN_HOSTS".into(),
            dir.join("known_hosts").to_str().unwrap().into(),
        );
        let remote = TvSsh::connect(&config, false).unwrap();
        assert!(remote.command("true").unwrap().is_empty());
        assert!(remote.command("false").is_err());
        let bytes = vec![0xa5; 256 * 1024];
        remote.upload("/fixture with spaces", &bytes).unwrap();
        assert_eq!(remote.command("read-upload").unwrap(), bytes);
        assert!(remote
            .luna(
                "luna://fixture-install",
                &serde_json::json!({}),
                Some("INSTALLED")
            )
            .is_ok());
        assert!(remote
            .luna(
                "luna://fixture-fail",
                &serde_json::json!({}),
                Some("INSTALLED")
            )
            .is_err());
        drop(remote);
        config.values.insert(
            "TV_SSH_KNOWN_HOSTS".into(),
            dir.join("untrusted").to_str().unwrap().into(),
        );
        assert!(TvSsh::connect(&config, false).is_err());
    }

    #[test]
    fn install_needs_terminal_success_not_progress() {
        assert!(!response_complete(
            &json!({"returnValue":true,"details":{"state":"INSTALLING"}}),
            Some("INSTALLED")
        )
        .unwrap());
        assert!(
            response_complete(&json!({"details":{"state":"installed"}}), Some("INSTALLED"))
                .unwrap()
        );
        assert!(
            response_complete(&json!({"details":{"state":"SUCCESS"}}), Some("REMOVED")).unwrap()
        );
        assert!(response_complete(
            &json!({"returnValue":false,"details":{"state":"SUCCESS"}}),
            Some("INSTALLED")
        )
        .is_err());
        assert!(
            response_complete(&json!({"details":{"state":"FAILED"}}), Some("INSTALLED")).is_err()
        );
        assert!(!response_complete(
            &json!({"details":{"state":"NOT_INSTALLED"}}),
            Some("INSTALLED")
        )
        .unwrap());
    }
    #[test]
    fn shell_arguments_preserve_literal_metacharacters() {
        assert_eq!(quote("a'b;$HOME"), "'a'\\''b;$HOME'");
    }
    #[test]
    fn ssh_paths_are_per_tv_and_bad_endpoints_fail() {
        let root = tempfile::tempdir().unwrap();
        let mut a = crate::common::test_config(root.path());
        let mut b = a.clone();
        b.tv = "bedroom".into();
        assert_ne!(key_path(&a), key_path(&b));
        assert_ne!(known_hosts_path(&a), known_hosts_path(&b));
        a.values.insert("TV_SSH_USER".into(), "-oBad".into());
        assert!(endpoint(&a).is_err());
        a.values.remove("TV_SSH_USER");
        a.values.insert("TV_SSH_PORT".into(), "0".into());
        assert!(endpoint(&a).is_err());
    }
}
