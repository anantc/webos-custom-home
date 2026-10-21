use crate::common::{fault, matches, private_write, Config, Http, Result};
use serde_json::Value;
use std::{fs, time::Duration};
use tokio::time::sleep;

pub fn valid_token(value: &str) -> Result<String> {
    if !matches(r"^[A-Za-z0-9]{1,4096}$", value) {
        return Err(fault("Developer Mode token is missing or invalid."));
    }
    Ok(value.into())
}
fn cached_token(config: &Config, fresh: Result<String>) -> Result<String> {
    let cache = config.state_dir().join("devmode-token");
    match fresh {
        Ok(token) => {
            let token = valid_token(&token)?;
            private_write(&cache, token.as_bytes())?;
            Ok(token)
        }
        Err(_) => valid_token(
            fs::read_to_string(cache)
                .map_err(|_| {
                    fault("No usable token. Turn on the TV and check SSH, or set TV_DEVMODE_TOKEN.")
                })?
                .trim(),
        ),
    }
}
pub async fn get_token(config: &Config) -> Result<String> {
    if let Some(value) = config.get("TV_DEVMODE_TOKEN") {
        return valid_token(value.trim());
    }
    cached_token(config, crate::ssh::read_token(config).await)
}
pub fn remaining_seconds(value: &Value) -> Result<u64> {
    if value["result"] != "success" {
        return Err(fault("LG did not confirm session validity."));
    }
    let text = value["errorMsg"].as_str().unwrap_or("");
    if !matches(r"^\d{1,6}:[0-5]\d:[0-5]\d$", text) {
        return Err(fault("LG returned an unrecognized remaining-time value."));
    }
    let fields: Vec<u64> = text
        .split(':')
        .map(|n| n.parse().expect("validated decimal"))
        .collect();
    let seconds = fields[0] * 3600 + fields[1] * 60 + fields[2];
    if seconds < 72 * 3600 {
        return Err(fault(
            "Developer Mode has less than 72 hours remaining after renewal.",
        ));
    }
    Ok(seconds)
}
trait LgService {
    async fn request(&self, method: &str, token: &str) -> Result<Value>;
}
impl LgService for Http {
    async fn request(&self, method: &str, token: &str) -> Result<Value> {
        // Methods are internal constants; tokens are strictly alphanumeric and never logged.
        let url = format!(
            "https://developer.lge.com/secure/{method}.dev?sessionToken={}",
            valid_token(token)?
        );
        let value: Value = serde_json::from_slice(&self.request(&url, None).await?)
            .map_err(|_| fault("LG returned invalid JSON."))?;
        if value["result"] != "success" {
            return Err(fault("LG did not confirm the session request."));
        }
        Ok(value)
    }
}
async fn renew_with(service: &impl LgService, token: &str) -> Result<u64> {
    let reset = service.request("ResetDevModeSession", token).await?;
    // LG legitimately returns errorCode=200 and errorMsg=GNL for successful resets.
    if reset["result"] != "success" {
        return Err(fault("LG did not confirm renewal."));
    }
    remaining_seconds(&service.request("CheckDevModeSession", token).await?)
}
pub async fn run(config: &Config) -> Result<()> {
    let url = config.ping_url("HC_RENEWAL_URL", false)?;
    let http = Http::new()?;
    http.ping(&url, "/start", "Developer Mode renewal started.")
        .await;
    for attempt in 0..3 {
        let result = match get_token(config).await {
            Ok(token) => renew_with(&http, &token).await,
            Err(e) => Err(e),
        };
        match result {
            Ok(seconds) => {
                crate::log(&format!(
                    "LG confirms {} hours of Developer Mode remaining.",
                    seconds / 3600
                ));
                if http
                    .ping(
                        &url,
                        "",
                        &format!(
                            "Renewed and verified with LG; {} hours remain.",
                            seconds / 3600
                        ),
                    )
                    .await
                {
                    return Ok(());
                }
                return Err(fault("Renewal verified, but its Healthchecks ping failed."));
            }
            Err(e) => crate::log(&e.to_string()),
        }
        if attempt < 2 {
            sleep(Duration::from_secs(15)).await;
        }
    }
    http.ping(
        &url,
        "/fail",
        "Developer Mode renewal or validity verification failed. Check host logs.",
    )
    .await;
    Err(fault("Developer Mode renewal failed after three attempts."))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::Mutex;
    struct Fake {
        replies: Mutex<Vec<Value>>,
        calls: Mutex<Vec<String>>,
    }
    impl LgService for Fake {
        async fn request(&self, method: &str, _token: &str) -> Result<Value> {
            self.calls.lock().unwrap().push(method.into());
            Ok(self.replies.lock().unwrap().remove(0))
        }
    }
    #[tokio::test]
    async fn gnl_success_is_followed_by_independent_verification() {
        let service = Fake {
            replies: Mutex::new(vec![
                json!({"result":"success","errorCode":"200","errorMsg":"GNL"}),
                json!({"result":"success","errorMsg":"999:59:59"}),
            ]),
            calls: Mutex::new(vec![]),
        };
        assert_eq!(
            renew_with(&service, "fixture").await.unwrap(),
            1000 * 3600 - 1
        );
        assert_eq!(
            *service.calls.lock().unwrap(),
            vec!["ResetDevModeSession", "CheckDevModeSession"]
        );
    }
    #[tokio::test]
    async fn reset_failure_never_reaches_success_check() {
        let service = Fake {
            replies: Mutex::new(vec![json!({"result":"fail"})]),
            calls: Mutex::new(vec![]),
        };
        assert!(renew_with(&service, "fixture").await.is_err());
        assert_eq!(service.calls.lock().unwrap().len(), 1);
    }
    #[test]
    fn invalid_short_or_expired_lifetimes_fail_closed() {
        for text in ["GNL", "0:00:00", "71:59:59", "999:60:00", ""] {
            assert!(remaining_seconds(&json!({"result":"success","errorMsg":text})).is_err());
        }
        assert_eq!(
            remaining_seconds(&json!({"result":"success","errorMsg":"72:00:00"})).unwrap(),
            72 * 3600
        );
    }
    #[test]
    fn fresh_token_is_cached_and_used_during_standby() {
        let dir = tempfile::tempdir().unwrap();
        let c = crate::common::test_config(dir.path());
        assert_eq!(cached_token(&c, Ok("first".into())).unwrap(), "first");
        assert_eq!(cached_token(&c, Err(fault("TV off."))).unwrap(), "first");
        assert_eq!(cached_token(&c, Ok("second".into())).unwrap(), "second");
        assert_eq!(cached_token(&c, Err(fault("TV off."))).unwrap(), "second");
    }
    #[tokio::test]
    async fn direct_token_needs_no_ssh_or_network() {
        let dir = tempfile::tempdir().unwrap();
        let mut c = crate::common::test_config(dir.path());
        c.values
            .insert("TV_DEVMODE_TOKEN".into(), "directToken".into());
        assert_eq!(get_token(&c).await.unwrap(), "directToken");
        assert!(!c.state_dir().exists());
    }
}
