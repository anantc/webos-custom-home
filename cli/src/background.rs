use crate::{
    common::{fault, Config, Error, FileLock, Result},
    install,
};
use std::time::Duration;
use tokio::time::{sleep, Instant};

trait Jobs {
    fn installed(&self, config: &Config) -> bool;
    async fn stop(&self, config: &Config) -> Result<()>;
    async fn prepare(&self, config: &Config) -> Result<()>;
    async fn start(&self, config: &Config) -> Result<()>;
}

struct SystemJobs;
impl Jobs for SystemJobs {
    fn installed(&self, config: &Config) -> bool {
        ["watch", "renew"].iter().any(|job| {
            let path = if cfg!(target_os = "macos") {
                config.home.join(format!(
                    "Library/LaunchAgents/{}.plist",
                    config.mac_label(job)
                ))
            } else {
                config
                    .home
                    .join(".config/systemd/user")
                    .join(config.linux_unit(job, "service"))
            };
            path.exists()
        })
    }
    async fn stop(&self, config: &Config) -> Result<()> {
        install::stop_jobs(config).await
    }
    async fn prepare(&self, config: &Config) -> Result<()> {
        if config.get("TV_RENEW_ENABLED") == Some("1") {
            println!("Checking Developer Mode renewal before scheduling it.");
            crate::renew::run(config).await?;
        }
        if config.get("TV_WATCH_ENABLED") == Some("1") {
            let mut peer = crate::tv::connect(config, true).await?;
            peer.close().await;
            let template = crate::package::Package::read(crate::package::bundled())?;
            crate::inventory::update(config, &template).await?;
        }
        Ok(())
    }
    async fn start(&self, config: &Config) -> Result<()> {
        install::install(config).await
    }
}

async fn save_when_idle(config: &Config) -> Result<()> {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        match config.configure(true) {
            Err(Error::Busy) if Instant::now() < deadline => {
                sleep(Duration::from_millis(100)).await
            }
            result => return result,
        }
    }
}

async fn replace<J: Jobs>(
    previous: &Config,
    next: &Config,
    jobs: &J,
    interrupted: impl std::future::Future<Output = ()>,
) -> Result<()> {
    let _lock = FileLock::acquire(&previous.state_dir().join("background.lock"))?;
    let current = Config::load_at(previous.home.clone(), &previous.tv, false, [])?;
    if current.values != previous.values {
        return Err(fault("This TV's settings changed during setup. Run background again to use the latest settings."));
    }
    let restore_jobs = jobs.installed(previous);
    let apply = tokio::select! {
        result = async {
        jobs.stop(previous).await?;
        save_when_idle(next).await?;
        jobs.prepare(next).await?;
        jobs.start(next).await
        } => result,
        () = interrupted => Err(fault("Background setup interrupted.")),
    };
    if let Err(error) = apply {
        crate::log(&format!("Could not apply new background settings: {error}"));
        let rollback = async {
            jobs.stop(next).await?;
            save_when_idle(previous).await?;
            if restore_jobs {
                jobs.start(previous).await?;
            }
            Ok::<_, Error>(())
        }
        .await;
        if let Err(rollback_error) = rollback {
            crate::log(&format!("Background recovery failed: {rollback_error}"));
            return Err(fault("Could not apply the new settings or fully restore the previous jobs. Check the service-manager error above before retrying."));
        }
        crate::log("Previous background settings restored.");
        return Err(error);
    }
    Ok(())
}

pub async fn apply(previous: &Config, next: &Config) -> Result<()> {
    // Reject invalid choices before stopping any working jobs.
    install::validate(next)?;
    println!("Applying settings and restarting this TV's background jobs.");
    replace(previous, next, &SystemJobs, async {
        let _ = tokio::signal::ctrl_c().await;
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    struct FakeJobs {
        present: bool,
        fail: &'static str,
        events: Mutex<Vec<String>>,
    }
    impl FakeJobs {
        fn record(&self, step: &str, config: &Config) {
            let saved = Config::load_at(config.home.clone(), &config.tv, false, []).unwrap();
            self.events
                .lock()
                .unwrap()
                .push(format!("{step}:{}", saved.get("TV_RENEW_ENABLED").unwrap()));
        }
    }
    impl Jobs for FakeJobs {
        fn installed(&self, _: &Config) -> bool {
            self.present
        }
        async fn stop(&self, config: &Config) -> Result<()> {
            self.record("stop", config);
            Ok(())
        }
        async fn prepare(&self, config: &Config) -> Result<()> {
            self.record("prepare", config);
            if self.fail == "wait" {
                std::future::pending::<()>().await;
            }
            if self.fail == "prepare" {
                Err(fault("TV check failed"))
            } else {
                Ok(())
            }
        }
        async fn start(&self, config: &Config) -> Result<()> {
            self.record("start", config);
            if self.fail == "start" && config.get("TV_RENEW_ENABLED") == Some("1") {
                Err(fault("Service start failed"))
            } else {
                Ok(())
            }
        }
    }
    fn configs(root: &std::path::Path) -> (Config, Config) {
        let mut previous = crate::common::test_config(root);
        previous
            .values
            .insert("TV_RENEW_ENABLED".into(), "0".into());
        previous.configure(false).unwrap();
        let mut next = previous.clone();
        next.values.insert("TV_RENEW_ENABLED".into(), "1".into());
        (previous, next)
    }
    #[tokio::test]
    async fn settings_are_saved_only_after_stopping_and_new_jobs_use_them() {
        let dir = tempfile::tempdir().unwrap();
        let (previous, next) = configs(dir.path());
        let jobs = FakeJobs {
            present: true,
            fail: "",
            events: Mutex::default(),
        };
        replace(&previous, &next, &jobs, std::future::pending())
            .await
            .unwrap();
        assert_eq!(
            *jobs.events.lock().unwrap(),
            ["stop:0", "prepare:1", "start:1"]
        );
    }
    #[tokio::test]
    async fn failed_apply_restores_settings_and_previous_jobs() {
        for failure in ["prepare", "start"] {
            let dir = tempfile::tempdir().unwrap();
            let (previous, next) = configs(dir.path());
            let jobs = FakeJobs {
                present: true,
                fail: failure,
                events: Mutex::default(),
            };
            assert!(replace(&previous, &next, &jobs, std::future::pending())
                .await
                .is_err());
            let saved = Config::load_at(previous.home.clone(), &previous.tv, false, []).unwrap();
            assert_eq!(saved.values, previous.values);
            let events = jobs.events.lock().unwrap();
            assert_eq!(events[events.len() - 2..], ["stop:1", "start:0"]);
        }
    }
    #[tokio::test]
    async fn first_setup_failure_does_not_start_previously_absent_jobs() {
        let dir = tempfile::tempdir().unwrap();
        let (previous, next) = configs(dir.path());
        let jobs = FakeJobs {
            present: false,
            fail: "prepare",
            events: Mutex::default(),
        };
        assert!(replace(&previous, &next, &jobs, std::future::pending())
            .await
            .is_err());
        assert_eq!(
            *jobs.events.lock().unwrap(),
            ["stop:0", "prepare:1", "stop:1"]
        );
    }
    #[tokio::test(start_paused = true)]
    async fn interrupted_apply_restores_settings_and_restarts_previous_jobs() {
        let dir = tempfile::tempdir().unwrap();
        let (previous, next) = configs(dir.path());
        let jobs = FakeJobs {
            present: true,
            fail: "wait",
            events: Mutex::default(),
        };
        let interrupted = sleep(Duration::from_secs(1));
        assert!(replace(&previous, &next, &jobs, interrupted).await.is_err());
        let saved = Config::load_at(previous.home.clone(), &previous.tv, false, []).unwrap();
        assert_eq!(saved.values, previous.values);
        assert_eq!(
            *jobs.events.lock().unwrap(),
            ["stop:0", "prepare:1", "stop:1", "start:0"]
        );
    }
    #[tokio::test]
    async fn concurrent_edit_is_preserved_without_stopping_jobs() {
        let dir = tempfile::tempdir().unwrap();
        let (previous, next) = configs(dir.path());
        next.configure(true).unwrap();
        let jobs = FakeJobs {
            present: true,
            fail: "",
            events: Mutex::default(),
        };
        assert!(replace(&previous, &next, &jobs, std::future::pending())
            .await
            .is_err());
        assert!(jobs.events.lock().unwrap().is_empty());
    }
    #[tokio::test(start_paused = true)]
    async fn shutdown_waits_for_the_old_process_to_release_its_lock() {
        let dir = tempfile::tempdir().unwrap();
        let (_, next) = configs(dir.path());
        let guard = FileLock::acquire(&next.state_dir().join("watch.lock")).unwrap();
        let release = async {
            sleep(Duration::from_millis(500)).await;
            drop(guard);
        };
        let (saved, ()) = tokio::join!(save_when_idle(&next), release);
        saved.unwrap();
    }
}
