use std::fs::{self, OpenOptions};
use std::path::PathBuf;
use std::process::{Command, Stdio};

use anyhow::{Context, Result};
use clap::Parser;

use voxd::config::{default_config_path, default_state_dir, load_or_init};

#[derive(Parser, Debug)]
#[command(name = "voxd", version, about = "Multi-provider TTS and STS daemon")]
struct Args {
    /// Run as a detached background daemon (setsid + log file).
    #[arg(long)]
    daemon: bool,
    /// Path to config.toml.
    #[arg(long, value_name = "PATH")]
    config: Option<PathBuf>,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_writer(std::io::stderr)
        .init();

    let args = Args::parse();
    let cfg_path = args.config.unwrap_or_else(default_config_path);
    let cfg = load_or_init(&cfg_path)?;

    if args.daemon {
        return spawn_detached(&cfg_path);
    }

    // Foreground: write pidfile, serve, clean up on shutdown.
    let pid_path = cfg.pid_file();
    if let Some(parent) = pid_path.parent() {
        fs::create_dir_all(parent).ok();
    }
    fs::write(&pid_path, std::process::id().to_string())
        .with_context(|| format!("write pidfile {}", pid_path.display()))?;

    let run = voxd::server::run(cfg).await;
    let _ = fs::remove_file(&pid_path);
    run
}

fn spawn_detached(cfg_path: &PathBuf) -> Result<()> {
    let exe = std::env::current_exe().context("current_exe")?;
    let state_dir = default_state_dir();
    fs::create_dir_all(&state_dir).ok();
    let log_path = state_dir.join("voxd.log");
    let log = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .with_context(|| format!("open log {}", log_path.display()))?;
    let log_err = log.try_clone().context("clone log handle")?;

    // `setsid -f` forks a fresh session and execs our binary in foreground mode,
    // which writes its own pidfile and binds the port. This process then exits.
    let child = Command::new("setsid")
        .arg("-f")
        .arg(&exe)
        .arg("--config")
        .arg(cfg_path)
        .stdin(Stdio::null())
        .stdout(Stdio::from(log))
        .stderr(Stdio::from(log_err))
        .spawn();

    match child {
        Ok(_) => {
            println!("voxd daemon started (logs: {})", log_path.display());
            Ok(())
        }
        Err(e) => {
            // Fallback: run in the foreground if setsid is unavailable.
            eprintln!("setsid failed ({e}); running in foreground");
            Ok(())
        }
    }
}
