use crate::common::{fault, private_dir, private_write, Config, Result};
use std::{fs, os::unix::fs::PermissionsExt, path::Path, time::Duration};
use tokio::{
    process::Command,
    time::{sleep, timeout, Instant},
};
fn linux_timer(config: &Config) -> String {
    format!("[Unit]\nDescription=Renew LG Developer Mode for {} daily\n\n[Timer]\nOnBootSec=2min\nOnUnitActiveSec=24h\nUnit={}\n\n[Install]\nWantedBy=timers.target\n", config.tv, config.linux_unit("renew", "service"))
}
fn xml(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}
fn args(config: &Config, command: &str, logging: bool) -> Vec<String> {
    let mut args = vec![
        config.installed_binary().to_string_lossy().into(),
        "--tv".into(),
        config.tv.clone(),
    ];
    if logging {
        args.extend([
            "--log-file".into(),
            config
                .state_dir()
                .join(format!("{command}.log"))
                .to_string_lossy()
                .into(),
        ]);
    }
    args.push(command.into());
    args
}
fn mac_plist(config: &Config, command: &str) -> String {
    let label = config.mac_label(command);
    let arguments = args(config, command, true)
        .iter()
        .map(|s| format!("<string>{}</string>", xml(s)))
        .collect::<String>();
    let schedule = if command == "watch" {
        "<key>KeepAlive</key><true/><key>ThrottleInterval</key><integer>20</integer>"
    } else {
        "<key>StartInterval</key><integer>86400</integer>"
    };
    let errors = xml(&config
        .state_dir()
        .join("launchd-errors.log")
        .to_string_lossy());
    format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\"><dict><key>Label</key><string>{label}</string><key>ProgramArguments</key><array>{arguments}</array><key>RunAtLoad</key><true/><key>ProcessType</key><string>Background</string><key>Umask</key><integer>63</integer><key>StandardOutPath</key><string>/dev/null</string><key>StandardErrorPath</key><string>{errors}</string>{schedule}</dict></plist>\n")
}
fn unit_quote(value: &str) -> Result<String> {
    if value.contains(['\r', '\n', '\0']) {
        return Err(fault("Service paths cannot contain newlines or NUL."));
    }
    Ok(format!(
        "\"{}\"",
        value
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('%', "%%")
            .replace('$', "$$")
    ))
}
fn linux_service(config: &Config, command: &str) -> Result<String> {
    let arguments = args(config, command, false)
        .iter()
        .map(|s| unit_quote(s))
        .collect::<Result<Vec<_>>>()?
        .join(" ");
    let (kind, extra) = if command == "watch" {
        (
            "simple",
            "Restart=on-failure\nRestartSec=20\n\n[Install]\nWantedBy=default.target\n",
        )
    } else {
        ("oneshot", "TimeoutStartSec=300\n")
    };
    Ok(format!("[Unit]\nDescription=Native TV Home {command}\n\n[Service]\nType={kind}\nExecStart={arguments}\nUMask=0077\n{extra}"))
}
fn service_failure(program: &str, args: &[&str], output: &std::process::Output) -> String {
    let detail: String = String::from_utf8_lossy(&output.stderr)
        .trim()
        .chars()
        .filter(|c| !c.is_control() || *c == '\n' || *c == '\t')
        .take(2000)
        .collect();
    format!(
        "Service manager: {program} {} failed ({}): {}",
        args.join(" "),
        output.status,
        if detail.is_empty() {
            "no error detail returned"
        } else {
            &detail
        }
    )
}
async fn stop_mac_job(domain: &str, label: &str) -> Result<()> {
    let service = format!("{domain}/{label}");
    if !command("launchctl", &["print", &service], false)
        .await?
        .status
        .success()
    {
        return Ok(());
    }
    command("launchctl", &["bootout", &service], true).await?;
    let deadline = Instant::now() + Duration::from_secs(10);
    while command("launchctl", &["print", &service], false)
        .await?
        .status
        .success()
    {
        if Instant::now() >= deadline {
            return Err(fault(
                "macOS did not finish stopping the previous job. Retry after it exits.",
            ));
        }
        sleep(Duration::from_millis(100)).await;
    }
    Ok(())
}
async fn command(program: &str, args: &[&str], check: bool) -> Result<std::process::Output> {
    let output = timeout(
        Duration::from_secs(330),
        Command::new(program).args(args).kill_on_drop(true).output(),
    )
    .await
    .map_err(|_| fault("Service manager timed out."))?
    .map_err(|_| fault("Cannot run the service manager."))?;
    if check && !output.status.success() {
        crate::log(&service_failure(program, args, &output));
        return Err(fault(
            "Could not install/start jobs. Check the service manager and tvhome logs.",
        ));
    }
    Ok(output)
}
pub fn selected_jobs(config: &Config) -> Result<Vec<&'static str>> {
    let mut jobs = Vec::new();
    for (key, job) in [("TV_WATCH_ENABLED", "watch"), ("TV_RENEW_ENABLED", "renew")] {
        match config.get(key) {
            Some("1") => jobs.push(job),
            Some("0") | None => (),
            _ => {
                return Err(fault(
                    "TV_WATCH_ENABLED and TV_RENEW_ENABLED must be 0 or 1.",
                ))
            }
        }
    }
    if jobs.is_empty() {
        return Err(fault(
            "Choose background jobs first with tvhome --tv NAME background.",
        ));
    }
    Ok(jobs)
}
fn linux_units(config: &Config, jobs: &[&str]) -> Result<Vec<(String, String)>> {
    let mut units = Vec::new();
    for job in jobs {
        units.push((
            config.linux_unit(job, "service"),
            linux_service(config, job)?,
        ));
        if *job == "renew" {
            units.push((config.linux_unit("renew", "timer"), linux_timer(config)));
        }
    }
    Ok(units)
}
pub(crate) fn validate(config: &Config) -> Result<()> {
    let jobs = selected_jobs(config)?;
    config.host()?;
    config.check_unique()?;
    for job in &jobs {
        config.ping_url(
            if *job == "watch" {
                "HC_WATCHER_URL"
            } else {
                "HC_RENEWAL_URL"
            },
            false,
        )?;
    }
    if jobs.contains(&"watch") {
        config.launcher()?;
        if config.key().is_none() {
            return Err(fault(
                "Run tvhome --tv NAME pair before installing Home watching.",
            ));
        }
        crate::ssh::validate(config)?;
        crate::package::Package::read(crate::package::bundled())?.policy()?;
    }
    if jobs.contains(&"renew") {
        if let Some(token) = config.get("TV_DEVMODE_TOKEN") {
            crate::renew::valid_token(token.trim())?;
        } else if !config.state_dir().join("devmode-token").is_file() {
            crate::ssh::validate(config)?;
        }
    }
    Ok(())
}
pub async fn install(config: &Config) -> Result<()> {
    validate(config)?;
    let jobs = selected_jobs(config)?;
    if !cfg!(any(target_os = "linux", target_os = "macos")) {
        return Err(fault("The installer supports macOS and systemd Linux."));
    }
    if cfg!(target_os = "linux") {
        let uid = unsafe { libc::getuid() }.to_string();
        let result = command(
            "loginctl",
            &["show-user", &uid, "--property=Linger", "--value"],
            false,
        )
        .await?;
        if !result.status.success() || String::from_utf8_lossy(&result.stdout).trim() != "yes" {
            return Err(fault("Enable user services with: sudo loginctl enable-linger \"$USER\"; then rerun install."));
        }
        command("systemctl", &["--user", "show-environment"], true).await?;
    }
    // Precompute and validate service files before changing an existing installation.
    let units_to_write = linux_units(config, &jobs)?;
    let executable =
        std::env::current_exe().map_err(|_| fault("Cannot locate this executable."))?;
    private_write(
        &config.installed_binary(),
        &fs::read(executable).map_err(|_| fault("Cannot read this executable."))?,
    )?;
    fs::set_permissions(config.installed_binary(), fs::Permissions::from_mode(0o700))
        .map_err(|_| fault("Cannot mark installed tvhome executable."))?;
    private_dir(&config.state_dir())?;
    if cfg!(target_os = "macos") {
        let domain = format!("gui/{}", unsafe { libc::getuid() });
        for name in ["watch", "renew"] {
            let label = config.mac_label(name);
            let path = config
                .home
                .join(format!("Library/LaunchAgents/{label}.plist"));
            stop_mac_job(&domain, &label).await?;
            if !jobs.contains(&name) {
                remove(&path)?;
                continue;
            }
            private_write(&path, mac_plist(config, name).as_bytes())?;
            command(
                "launchctl",
                &["bootstrap", &domain, &path.to_string_lossy()],
                true,
            )
            .await?;
        }
    } else {
        let watch = config.linux_unit("watch", "service");
        let renew = config.linux_unit("renew", "service");
        let timer = config.linux_unit("renew", "timer");
        command(
            "systemctl",
            &["--user", "disable", "--now", &watch, &timer],
            false,
        )
        .await?;
        command("systemctl", &["--user", "stop", &renew], false).await?;
        let units = config.home.join(".config/systemd/user");
        for name in [&watch, &renew, &timer] {
            if !units_to_write.iter().any(|(selected, _)| selected == name) {
                remove(&units.join(name))?;
            }
        }
        for (name, content) in units_to_write {
            private_write(&units.join(name), content.as_bytes())?;
        }
        command("systemctl", &["--user", "daemon-reload"], true).await?;
        for job in &jobs {
            let unit = if *job == "watch" { &watch } else { &timer };
            command("systemctl", &["--user", "enable", unit], true).await?;
            command("systemctl", &["--user", "restart", unit], true).await?;
        }
        if jobs.contains(&"renew") {
            command("systemctl", &["--user", "start", &renew], true).await?;
        }
    }
    if cfg!(target_os = "macos") {
        crate::log("Selected jobs start automatically when you log in after a restart. Keep this Mac awake.");
    } else {
        crate::log("Selected jobs start automatically after boot. Keep this computer awake.");
    }
    if jobs.contains(&"watch") {
        crate::log("Home watching installed; app catalogs refresh roughly once a minute.");
    }
    if jobs.contains(&"renew") {
        crate::log("Developer Mode renewal installed; runs now and daily.");
    }
    if config.get("HC_WATCHER_URL").is_some() || config.get("HC_RENEWAL_URL").is_some() {
        crate::log("Verify the enabled Healthchecks checks turn Up.");
    }

    Ok(())
}
fn remove(path: &Path) -> Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err(fault("Cannot remove a service file.")),
    }
}
pub(crate) async fn stop_jobs(config: &Config) -> Result<()> {
    if cfg!(target_os = "macos") {
        for name in ["watch", "renew"] {
            let label = config.mac_label(name);
            stop_mac_job(&format!("gui/{}", unsafe { libc::getuid() }), &label).await?;
            remove(
                &config
                    .home
                    .join(format!("Library/LaunchAgents/{label}.plist")),
            )?;
        }
    } else if cfg!(target_os = "linux") {
        let watch = config.linux_unit("watch", "service");
        let renew = config.linux_unit("renew", "service");
        let timer = config.linux_unit("renew", "timer");
        command(
            "systemctl",
            &["--user", "disable", "--now", &watch, &timer],
            false,
        )
        .await?;
        command("systemctl", &["--user", "stop", &renew], false).await?;
        for name in [&watch, &renew, &timer] {
            remove(&config.home.join(".config/systemd/user").join(name))?;
        }
        command("systemctl", &["--user", "daemon-reload"], true).await?;
    } else {
        return Err(fault("Unsupported service manager."));
    }
    Ok(())
}
pub async fn uninstall(config: &Config) -> Result<()> {
    stop_jobs(config).await?;
    crate::log("Jobs removed. Private configuration, keys and executable retained. Pause any configured Healthchecks checks.");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn service_error_includes_failed_operation_and_os_detail() {
        use std::os::unix::process::ExitStatusExt;
        let output = std::process::Output {
            status: std::process::ExitStatus::from_raw(5 << 8),
            stdout: b"not logged".to_vec(),
            stderr: b"Bootstrap failed: 5: Input/output error\n".to_vec(),
        };
        let message = service_failure(
            "launchctl",
            &["bootstrap", "gui/501", "/tmp/job.plist"],
            &output,
        );
        assert!(message.contains("launchctl bootstrap gui/501 /tmp/job.plist"));
        assert!(message.contains("Bootstrap failed: 5: Input/output error"));
        assert!(!message.contains("not logged"));
    }
    #[test]
    fn mac_jobs_parse_and_contain_no_secret_values() {
        let mut c = crate::common::test_config(Path::new("/home/a & b"));
        c.values
            .insert("TV_DEVMODE_TOKEN".into(), "secretToken".into());
        for name in ["watch", "renew"] {
            let text = mac_plist(&c, name);
            let p: plist::Value = plist::from_bytes(text.as_bytes()).unwrap();
            let dict = p.as_dictionary().unwrap();
            assert!(!text.contains("secretToken"));
            assert!(!text.contains("fixture-key"));
            assert_eq!(dict["Umask"].as_unsigned_integer(), Some(63));
            assert!(dict["ProgramArguments"].as_array().unwrap()[0]
                .as_string()
                .unwrap()
                .ends_with("/tvhome"));
            if name == "watch" {
                assert_eq!(dict["KeepAlive"].as_boolean(), Some(true));
            } else {
                assert_eq!(dict["StartInterval"].as_unsigned_integer(), Some(86400));
            }
        }
    }
    #[test]
    fn linux_paths_escape_expansion_and_newlines_are_rejected() {
        assert_eq!(
            unit_quote("/path with/$value/100%").unwrap(),
            "\"/path with/$$value/100%%\""
        );
        assert!(unit_quote("bad\nExecStart=bad").is_err());
        let c = crate::common::test_config(Path::new("/home/test"));
        let unit = linux_service(&c, "watch").unwrap();
        assert!(!unit.contains("fixture-key"));
        assert!(unit.contains("--tv"));
        assert!(unit.contains("living-room"));
        assert!(unit.contains("Restart=on-failure"));
        assert!(linux_timer(&c).contains("OnUnitActiveSec=24h"));
    }
    #[test]
    fn job_selection_builds_only_opted_in_services_and_accepts_no_monitoring() {
        let dir = tempfile::tempdir().unwrap();
        let mut config = crate::common::test_config(dir.path());
        assert!(selected_jobs(&config).is_err());
        config
            .values
            .insert("TV_DEVMODE_TOKEN".into(), "fixtureToken".into());
        config.values.insert("TV_RENEW_ENABLED".into(), "1".into());
        config.values.remove("TV_CLIENT_KEY");
        assert!(
            validate(&config).is_ok(),
            "renewal needs neither pairing nor Healthchecks"
        );
        let jobs = selected_jobs(&config).unwrap();
        assert_eq!(jobs, vec!["renew"]);
        let units = linux_units(&config, &jobs).unwrap();
        assert_eq!(units.len(), 2);
        assert!(units.iter().all(|(name, _)| name.contains("renew")));
        config.values.insert("TV_RENEW_ENABLED".into(), "0".into());
        config.values.insert("TV_WATCH_ENABLED".into(), "1".into());
        let units = linux_units(&config, &selected_jobs(&config).unwrap()).unwrap();
        assert_eq!(units.len(), 1);
        assert!(units[0].0.contains("watch"));
        config.values.insert("TV_RENEW_ENABLED".into(), "1".into());
        assert_eq!(
            linux_units(&config, &selected_jobs(&config).unwrap())
                .unwrap()
                .len(),
            3
        );
        config
            .values
            .insert("TV_RENEW_ENABLED".into(), "yes".into());
        assert!(selected_jobs(&config).is_err());
    }
    #[test]
    fn incomplete_configuration_never_qualifies_for_install() {
        let c = crate::common::test_config(Path::new("/tmp"));
        assert!(validate(&c).is_err());
    }
}

#[cfg(test)]
mod multi_tv_tests {
    use super::*;
    #[test]
    fn service_names_arguments_and_timers_target_only_the_selected_tv() {
        let root = tempfile::tempdir().unwrap();
        let a = crate::common::test_config(root.path());
        let mut b = a.clone();
        b.tv = "bedroom".into();
        for c in [&a, &b] {
            for command in ["watch", "renew"] {
                let p: plist::Value = plist::from_bytes(mac_plist(c, command).as_bytes()).unwrap();
                let p = p.as_dictionary().unwrap();
                assert_eq!(p["Label"].as_string().unwrap(), c.mac_label(command));
                let args = p["ProgramArguments"].as_array().unwrap();
                assert_eq!(args[1].as_string(), Some("--tv"));
                assert_eq!(args[2].as_string(), Some(c.tv.as_str()));
                let unit = linux_service(c, command).unwrap();
                assert!(unit.contains(&format!("\"--tv\" \"{}\"", c.tv)));
                assert!(unit.contains(c.installed_binary().to_str().unwrap()));
            }
            assert!(
                linux_timer(c).contains(&format!("Unit={}\n", c.linux_unit("renew", "service")))
            );
        }
        assert_ne!(a.mac_label("watch"), b.mac_label("watch"));
        assert_ne!(
            a.linux_unit("renew", "timer"),
            b.linux_unit("renew", "timer")
        );
        // Removing one profile's job file must leave the other runnable.
        let jobs = root.path().join(".config/systemd/user");
        for c in [&a, &b] {
            private_write(
                &jobs.join(c.linux_unit("watch", "service")),
                linux_service(c, "watch").unwrap().as_bytes(),
            )
            .unwrap();
        }
        remove(&jobs.join(a.linux_unit("watch", "service"))).unwrap();
        assert!(jobs.join(b.linux_unit("watch", "service")).is_file());
    }
}
