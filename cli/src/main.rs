mod common;
mod install;
mod inventory;
mod package;
mod renew;
mod self_install;
mod setup;
mod ssh;
mod sync;
mod tv;
mod watch;

use clap::{Parser, Subcommand};
use common::{fault, Config, Result};
use std::{
    fs,
    io::Write,
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Parser)]
#[command(
    name = "tvhome",
    version,
    arg_required_else_help = true,
    about = "Pair, sync and monitor native Home on LG webOS"
)]
struct Cli {
    /// Install this executable, license notices and shell PATH for your user
    #[arg(long, exclusive = true)]
    install: bool,
    /// Print the license notices embedded in this executable
    #[arg(long, exclusive = true)]
    licenses: bool,
    /// Named TV profile (for example living-room or bedroom)
    #[arg(long)]
    tv: Option<String>,
    /// Rotating private log for launchd
    #[arg(long, global = true)]
    log_file: Option<PathBuf>,
    #[command(subcommand)]
    command: Option<Action>,
}
impl Cli {
    fn validate_action(&self) -> Result<()> {
        if (self.install || self.licenses) && self.command.is_some() {
            return Err(fault(
                "Use --install or --licenses on its own, without a TV command.",
            ));
        }
        Ok(())
    }
}
#[derive(Subcommand)]
enum Action {
    /// List saved TVs and their cached app counts
    Tvs {
        #[arg(long)]
        json: bool,
    },
    /// List this TV's installed apps; refresh also publishes changes to Home
    Apps {
        #[arg(long)]
        refresh: bool,
        #[arg(long)]
        json: bool,
    },
    /// Register a TV, retrieve its key, and install the bundled Home app
    Setup {
        /// TV IP address or hostname; prompted for a new profile if omitted
        #[arg(long)]
        host: Option<String>,
        /// Retrieve a fresh SSH key from the TV's enabled Key Server
        #[arg(long)]
        refresh_key: bool,
        /// Developer override: use a Home setup template built locally
        #[arg(long)]
        package: Option<PathBuf>,
    },
    /// Choose optional Home watching, session renewal and Healthchecks alerts
    Background,
    /// Remove Home from the TV (stop background jobs first)
    RemoveApp,
    /// Save this TV's environment variables as a private, isolated profile
    Configure {
        /// Replace the entire saved configuration for this TV
        #[arg(long)]
        replace: bool,
    },
    /// Authorize the connection and save its pairing key
    Pair,
    /// Export this TV's app catalog for package development
    Sync {
        #[arg(long)]
        list: bool,
    },
    /// Redirect LG Home and keep its app catalog current
    Watch,
    /// Renew Developer Mode now, then verify its remaining lifetime
    Renew,
    /// Install and start this TV's saved selection of background jobs
    Install,
    /// Stop and remove this TV's background jobs
    Uninstall,
}
static LOG_PATH: OnceLock<Mutex<PathBuf>> = OnceLock::new();
fn rotate(path: &Path) -> std::io::Result<()> {
    if fs::metadata(path).map(|m| m.len()).unwrap_or(0) < 2_000_000 {
        return Ok(());
    }
    for n in (1..=3).rev() {
        let from = if n == 1 {
            path.to_path_buf()
        } else {
            PathBuf::from(format!("{}.{}", path.display(), n - 1))
        };
        let to = PathBuf::from(format!("{}.{}", path.display(), n));
        if from.exists() {
            fs::rename(from, to)?;
        }
    }
    Ok(())
}
fn log(message: &str) {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    if let Some(path) = LOG_PATH.get() {
        let path = path.lock().unwrap();
        if rotate(&path).is_ok() {
            if let Ok(mut f) = fs::OpenOptions::new()
                .create(true)
                .append(true)
                .mode(0o600)
                .open(&*path)
            {
                if writeln!(f, "{timestamp} {message}").is_ok() {
                    return;
                }
            }
        }
    }
    eprintln!("{timestamp} {message}");
}
async fn dispatch(config: &Config, action: &Action) -> Result<()> {
    match action {
        Action::Tvs { .. } | Action::Setup { .. } | Action::Background => {
            unreachable!("handled before profile loading")
        }
        Action::RemoveApp => setup::remove(config).await,
        Action::Apps { refresh, json } => inventory::list_apps(config, *refresh, *json).await,
        Action::Configure { replace } => {
            config.configure(*replace)?;
            log(&format!("Saved TV profile {}.", config.tv));
            Ok(())
        }
        Action::Pair => {
            let mut peer = tv::connect(config, true).await?;
            peer.close().await;
            log("Pairing saved.");
            Ok(())
        }
        Action::Sync { list } => sync::run(config, &config.catalog_dir(), *list).await,
        Action::Watch => watch::run(config).await,
        Action::Renew => renew::run(config).await,
        Action::Install => install::install(config).await,
        Action::Uninstall => install::uninstall(config).await,
    }
}
async fn execute(cli: Cli) -> Result<()> {
    cli.validate_action()?;
    if cli.install {
        return self_install::run();
    }
    if cli.licenses {
        print!("{}", self_install::LICENSES);
        return Ok(());
    }
    let command = cli
        .command
        .ok_or(fault("Choose a command; see tvhome --help."))?;
    if let Action::Tvs { json } = &command {
        let home = std::env::var_os("HOME").ok_or(fault("HOME is unavailable."))?;
        return inventory::list_tvs(Path::new(&home), *json);
    }
    let tv = cli.tv.as_deref().ok_or(fault(
        "Choose a TV with --tv NAME. Use tvhome tvs to list saved TVs.",
    ))?;
    if let Action::Setup {
        host,
        refresh_key,
        package,
    } = &command
    {
        return setup::run(tv, host.as_deref(), *refresh_key, package.as_deref()).await;
    }
    if matches!(command, Action::Background) {
        return setup::background(tv).await;
    }
    let config = Config::load(tv, matches!(command, Action::Configure { .. }))?;
    // Acquire before failure reporting: a duplicate process is not a failed TV.
    let _operation_lock = match command {
        Action::Watch => Some(common::FileLock::acquire(
            &config.state_dir().join("watch.lock"),
        )?),
        Action::Renew => Some(common::FileLock::acquire(
            &config.state_dir().join("renew.lock"),
        )?),
        Action::Pair | Action::Sync { .. } | Action::RemoveApp => Some(common::FileLock::acquire(
            &config.state_dir().join("setup.lock"),
        )?),
        _ => None,
    };
    if let Some(path) = cli.log_file {
        common::private_dir(
            path.parent()
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or(Path::new(".")),
        )?;
        let _ = LOG_PATH.set(Mutex::new(path));
    }
    let result = tokio::select! {
        result=dispatch(&config,&command)=>result,
        signal=tokio::signal::ctrl_c()=>signal.map_err(|_|fault("Cannot listen for interruption.")),
    };
    if result.is_err() {
        let key = match command {
            Action::Watch => Some("HC_WATCHER_URL"),
            Action::Renew => Some("HC_RENEWAL_URL"),
            _ => None,
        };
        if let Some(key) = key {
            if let (Ok(url), Ok(http)) = (config.ping_url(key, false), common::Http::new()) {
                http.ping(
                    &url,
                    "/fail",
                    "tvhome failed. Check configuration and host logs.",
                )
                .await;
            }
        }
    }
    result
}
#[tokio::main]
async fn main() -> std::process::ExitCode {
    // Private files and rotated logs remain owner-only even outside the service manager.
    unsafe {
        libc::umask(0o077);
    }
    match execute(Cli::parse()).await {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            log(&e.to_string());
            std::process::ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod cli_tests {
    use super::*;
    #[test]
    fn self_install_needs_no_tv_and_rejects_other_operations() {
        assert!(
            Cli::try_parse_from(["tvhome", "--install"])
                .unwrap()
                .install
        );
        assert!(
            Cli::try_parse_from(["tvhome", "--licenses"])
                .unwrap()
                .licenses
        );
        for args in [
            vec!["tvhome", "--install", "--tv", "bedroom"],
            vec!["tvhome", "--install", "tvs"],
            vec!["tvhome", "--install", "--licenses"],
        ] {
            assert!(Cli::try_parse_from(args)
                .map(|cli| cli.validate_action().is_err())
                .unwrap_or(true));
        }
        assert!(Cli::try_parse_from(["tvhome"]).is_err());
    }
    #[test]
    fn tv_inventory_and_named_commands_have_help() {
        assert!(Cli::try_parse_from(["tvhome", "tvs"]).is_ok());
        let cli = Cli::try_parse_from(["tvhome", "--tv", "bedroom", "sync", "--list"]).unwrap();
        assert_eq!(cli.tv.as_deref(), Some("bedroom"));
        assert_eq!(
            Cli::try_parse_from(["tvhome", "--help"])
                .err()
                .unwrap()
                .kind(),
            clap::error::ErrorKind::DisplayHelp
        );
    }
}
