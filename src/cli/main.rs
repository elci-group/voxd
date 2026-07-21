use std::io::{IsTerminal, Read};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Context, Result};
use clap::{Parser, Subcommand};
use serde_json::{json, Value};

use voxd::config::{default_config_path, load_or_init, save_config};
use voxd::project;

#[derive(Parser, Debug)]
#[command(
    name = "voxd-cli",
    version,
    about = "Client for the voxd ElevenLabs TTS daemon"
)]
struct Args {
    /// Path to config.toml (also used to reach the daemon).
    #[arg(long, global = true, value_name = "PATH")]
    config: Option<PathBuf>,
    /// Print raw JSON responses.
    #[arg(long, global = true)]
    json: bool,
    #[command(subcommand)]
    cmd: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Speak text (default voice resolved by project, or --system).
    Speak {
        /// Text to speak; reads stdin when omitted.
        text: Vec<String>,
        /// Use the single unifying system voice (no project binding).
        #[arg(long)]
        system: bool,
        /// Project path or id (defaults to current directory).
        #[arg(long)]
        project: Option<String>,
        /// Override voice id for this call.
        #[arg(long)]
        voice: Option<String>,
        /// Tone label to record/override.
        #[arg(long)]
        label: Option<String>,
        /// Do not play audio locally.
        #[arg(long)]
        no_play: bool,
        /// Force fresh synthesis (ignore cache).
        #[arg(long)]
        no_cache: bool,
        /// Play on the server instead of locally.
        #[arg(long)]
        server_play: bool,
    },
    /// List available ElevenLabs voices.
    Voices,
    /// List project → voice bindings.
    Projects,
    /// Assign a voice (+ optional label) to a project.
    Assign {
        /// Project path or id.
        project: String,
        /// Voice id to bind.
        voice: String,
        /// Tone label.
        #[arg(long)]
        label: Option<String>,
    },
    /// Remove a project binding (aliases: forget).
    #[command(alias = "forget")]
    Unassign {
        /// Project path or id.
        project: String,
    },
    /// Control the always-listening keyword-activated STS loop.
    Listen {
        #[command(subcommand)]
        action: ListenAction,
    },
    /// Daemon status.
    Status,
    /// Show or update config.toml.
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },
    /// Daemon health (no auth).
    Health,
    /// Stop the daemon (SIGTERM via pidfile).
    Stop,
    /// Show recent daemon logs.
    Logs,
}

#[derive(Subcommand, Debug)]
enum ListenAction {
    /// Start the always-listening loop.
    Start,
    /// Stop the loop.
    Stop,
    /// Is the loop running?
    Status,
    /// Record N seconds from the mic, transcribe via the daemon, print text.
    Test {
        #[arg(long, default_value = "3")]
        secs: u64,
    },
}

#[derive(Subcommand, Debug)]
enum ConfigAction {
    /// Print the config path.
    Path,
    /// Create config.toml if needed and print it.
    Init,
    /// Print current config.
    Show,
    /// Update one config key and save config.toml.
    Set {
        /// Dotted key, e.g. defaults.stability or listen.wake_word.
        key: String,
        /// New value. Booleans accept true/false, yes/no, on/off, or 1/0.
        value: String,
    },
}

struct Api {
    base: String,
    token: String,
    http: reqwest::blocking::Client,
}

impl Api {
    fn new(bind: &str, token: &str) -> Result<Self> {
        let http = reqwest::blocking::Client::builder()
            .user_agent("voxd-cli/0.1")
            .timeout(Duration::from_secs(60))
            .build()
            .context("http client")?;
        Ok(Self {
            base: format!("http://{bind}"),
            token: token.to_string(),
            http,
        })
    }

    fn get(&self, path: &str, auth: bool) -> Result<Value> {
        let mut req = self.http.get(format!("{}{}", self.base, path));
        if auth {
            req = req.bearer_auth(&self.token);
        }
        send(req)
    }

    fn post(&self, path: &str, body: &Value) -> Result<Value> {
        let req = self
            .http
            .post(format!("{}{}", self.base, path))
            .bearer_auth(&self.token)
            .json(body);
        send(req)
    }

    fn post_raw(&self, path: &str, bytes: Vec<u8>, content_type: &str) -> Result<Value> {
        let req = self
            .http
            .post(format!("{}{}", self.base, path))
            .bearer_auth(&self.token)
            .header("Content-Type", content_type)
            .body(bytes);
        send(req)
    }

    fn health(&self) -> bool {
        self.get("/health", false)
            .ok()
            .and_then(|v| v.get("ok")?.as_bool())
            .unwrap_or(false)
    }
}

fn send(req: reqwest::blocking::RequestBuilder) -> Result<Value> {
    let resp = req.send().context("request")?;
    let status = resp.status();
    let body: Value = resp.json().unwrap_or_else(|_| json!({}));
    if !status.is_success() {
        let msg = body
            .get("error")
            .and_then(|e| e.as_str())
            .unwrap_or(status.as_str());
        bail!("HTTP {status}: {msg}");
    }
    Ok(body)
}

fn main() -> Result<()> {
    // Terminate cleanly on a broken pipe (e.g. `voxd-cli projects | head`)
    // instead of panicking with a backtrace when stdout closes early.
    unsafe { libc::signal(libc::SIGPIPE, libc::SIG_DFL) };

    let args = Args::parse();
    let cfg_path = args.config.unwrap_or_else(default_config_path);
    let mut cfg = load_or_init(&cfg_path)?;
    let cmd = match args.cmd {
        Command::Config { action } => return handle_config(action, &cfg_path, &mut cfg, args.json),
        cmd => cmd,
    };

    let api = Api::new(&cfg.server.bind, &cfg.server.auth_token)?;

    // Auto-start the daemon for commands that need it.
    let needs_daemon = !matches!(cmd, Command::Stop | Command::Logs);
    if needs_daemon && !api.health() {
        start_daemon(&cfg_path)?;
        wait_health(&api)?;
    }

    match cmd {
        Command::Speak {
            text,
            system,
            project,
            voice,
            label,
            no_play,
            no_cache,
            server_play,
        } => {
            let t = gather_text(&text)?;
            let mut body =
                json!({ "text": t, "system": system, "no_cache": no_cache, "play": server_play });
            if let Some(p) = project {
                if looks_like_path(&p) {
                    body["project_path"] = json!(p);
                } else {
                    body["project_id"] = json!(p);
                }
            }
            if let Some(v) = voice {
                body["voice_id"] = json!(v);
            }
            if let Some(l) = label {
                body["label"] = json!(l);
            }
            let resp = api.post("/speak", &body)?;
            if args.json {
                println!("{}", serde_json::to_string_pretty(&resp)?);
            } else {
                println!(
                    "{} ({}){} · {} chars · {}",
                    resp["voice_id"].as_str().unwrap_or("?"),
                    resp["label"].as_str().unwrap_or("?"),
                    if resp["cached"].as_bool().unwrap_or(false) {
                        " [cached]"
                    } else {
                        ""
                    },
                    resp["chars"].as_u64().unwrap_or(0),
                    resp["audio_path"].as_str().unwrap_or("?"),
                );
            }
            if !no_play && !server_play {
                if let Some(p) = resp["audio_path"].as_str() {
                    if !p.is_empty() {
                        voxd::play::play_blocking(std::path::Path::new(p))?;
                    }
                }
            }
        }
        Command::Voices => {
            let v = api.get("/voices", true)?;
            print_json_or(&v, args.json, |v| {
                let arr = v.as_array().cloned().unwrap_or_default();
                println!("{:<38} {:<24} CATEGORY", "VOICE_ID", "NAME");
                for x in &arr {
                    println!(
                        "{:<38} {:<24} {}",
                        x["voice_id"].as_str().unwrap_or(""),
                        x["name"].as_str().unwrap_or(""),
                        x["category"].as_str().unwrap_or(""),
                    );
                }
                println!("{} voices", arr.len());
            });
        }
        Command::Projects => {
            let v = api.get("/projects", true)?;
            print_json_or(&v, args.json, |v| {
                let arr = v.as_array().cloned().unwrap_or_default();
                println!("{:<18} {:<24} {:<20} VOICE", "ID", "NAME", "LABEL");
                for x in &arr {
                    println!(
                        "{:<18} {:<24} {:<20} {}",
                        x["id"].as_str().unwrap_or(""),
                        x["name"].as_str().unwrap_or(""),
                        x["label"].as_str().unwrap_or(""),
                        x["voice_id"].as_str().unwrap_or(""),
                    );
                }
                println!("{} projects", arr.len());
            });
        }
        Command::Assign {
            project,
            voice,
            label,
        } => {
            let id = to_project_id(&project)?;
            let mut body = json!({ "project_id": id, "voice_id": voice });
            if looks_like_path(&project) {
                body["project_path"] = json!(project);
            }
            if let Some(l) = label {
                body["label"] = json!(l);
            }
            let resp = api.post("/projects/assign", &body)?;
            print_json_or(&resp, args.json, |r| {
                println!(
                    "assigned {} -> {} ({})",
                    r["id"].as_str().unwrap_or("?"),
                    r["voice_id"].as_str().unwrap_or(""),
                    r["label"].as_str().unwrap_or("")
                );
            });
        }
        Command::Unassign { project } => {
            let id = to_project_id(&project)?;
            let resp = api.post("/projects/unassign", &json!({ "project_id": id }))?;
            print_json_or(&resp, args.json, |r| {
                println!("unassigned {}", r["id"].as_str().unwrap_or("?"));
            });
        }
        Command::Listen { action } => match action {
            ListenAction::Start => {
                let v = api.post("/listen/start", &json!({}))?;
                print_json_or(&v, args.json, |_| println!("listener started"));
            }
            ListenAction::Stop => {
                let v = api.post("/listen/stop", &json!({}))?;
                print_json_or(&v, args.json, |v| {
                    println!("stopped: {}", v["stopped"].as_bool().unwrap_or(false))
                });
            }
            ListenAction::Status => {
                let v = api.get("/listen/status", true)?;
                print_json_or(&v, args.json, |v| {
                    println!("listening: {}", v["listening"].as_bool().unwrap_or(false))
                });
            }
            ListenAction::Test { secs } => listen_test(&api, secs, args.json)?,
        },
        Command::Status => {
            let v = api.get("/status", true)?;
            if args.json {
                println!("{}", serde_json::to_string_pretty(&v)?);
            } else {
                println!("uptime:        {}s", v["uptime_secs"].as_u64().unwrap_or(0));
                println!("projects:      {}", v["projects"].as_u64().unwrap_or(0));
                println!("utterances:    {}", v["utterances"].as_u64().unwrap_or(0));
                println!(
                    "cache:         {} bytes ({})",
                    v["cache_bytes"].as_u64().unwrap_or(0),
                    v["cache_dir"].as_str().unwrap_or("")
                );
                println!(
                    "key_present:   {}",
                    v["key_present"].as_bool().unwrap_or(false)
                );
                println!(
                    "system_voice:  {}",
                    v["system_voice"].as_str().unwrap_or("")
                );
                println!("model:         {}", v["model"].as_str().unwrap_or(""));
            }
        }
        Command::Config { .. } => unreachable!("handled before API setup"),
        Command::Health => {
            let v = api.get("/health", false)?;
            println!("{}", serde_json::to_string(&v)?);
        }
        Command::Stop => stop_daemon(&cfg)?,
        Command::Logs => show_logs(),
    }
    Ok(())
}

fn handle_config(
    action: ConfigAction,
    cfg_path: &PathBuf,
    cfg: &mut voxd::config::Config,
    raw: bool,
) -> Result<()> {
    match action {
        ConfigAction::Path => {
            if raw {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&json!({ "path": cfg_path }))?
                );
            } else {
                println!("{}", cfg_path.display());
            }
        }
        ConfigAction::Init | ConfigAction::Show => print_config(cfg, raw)?,
        ConfigAction::Set { key, value } => {
            cfg.set_key(&key, &value)?;
            save_config(cfg_path, cfg)?;
            print_config(cfg, raw)?;
        }
    }
    Ok(())
}

fn print_config(cfg: &voxd::config::Config, raw: bool) -> Result<()> {
    if raw {
        println!("{}", serde_json::to_string_pretty(cfg)?);
    } else {
        println!("{}", toml::to_string_pretty(cfg)?);
    }
    Ok(())
}

fn gather_text(parts: &[String]) -> Result<String> {
    if !parts.is_empty() {
        return Ok(parts.join(" "));
    }
    let mut buf = String::new();
    if !std::io::stdin().is_terminal() {
        std::io::stdin()
            .read_to_string(&mut buf)
            .context("read stdin")?;
    }
    if buf.trim().is_empty() {
        bail!("no text: pass arguments or pipe via stdin");
    }
    Ok(buf)
}

fn looks_like_path(s: &str) -> bool {
    s == "."
        || s == ".."
        || s.contains('/')
        || s.starts_with('~')
        || std::path::Path::new(s).exists()
}

fn to_project_id(s: &str) -> Result<String> {
    if looks_like_path(s) {
        let pref = project::resolve(std::path::Path::new(s))?;
        Ok(pref.id)
    } else {
        Ok(s.to_string())
    }
}

fn print_json_or<F: FnOnce(&Value)>(v: &Value, raw: bool, pretty: F) {
    if raw {
        println!("{}", serde_json::to_string_pretty(v).unwrap_or_default());
    } else {
        pretty(v);
    }
}

fn start_daemon(cfg_path: &PathBuf) -> Result<()> {
    let exe = std::env::current_exe().context("current_exe")?;
    // The daemon binary is the sibling `voxd` next to `voxd-cli`.
    let daemon = exe.with_file_name("voxd");
    let bin = if daemon.exists() {
        daemon
    } else {
        PathBuf::from("voxd")
    };
    std::process::Command::new(bin)
        .arg("--daemon")
        .arg("--config")
        .arg(cfg_path)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .context("spawn voxd --daemon")?;
    Ok(())
}

fn wait_health(api: &Api) -> Result<()> {
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(15) {
        if api.health() {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    Err(anyhow!("daemon did not become healthy within 15s"))
}

fn stop_daemon(cfg: &voxd::config::Config) -> Result<()> {
    let pid_path = cfg.pid_file();
    let raw = match std::fs::read_to_string(&pid_path) {
        Ok(s) => s,
        Err(_) => {
            println!("voxd is not running (no pidfile)");
            return Ok(());
        }
    };
    let pid: i32 = raw.trim().parse().context("parse pid")?;
    let rc = unsafe { libc::kill(pid, libc::SIGTERM) };
    if rc != 0 {
        bail!(
            "kill({pid}) failed (errno {})",
            std::io::Error::last_os_error()
        );
    }
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(10) {
        // Check if process still alive.
        let alive = unsafe { libc::kill(pid, 0) } == 0;
        if !alive {
            break;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    let _ = std::fs::remove_file(&pid_path);
    println!("voxd stopped (pid {pid})");
    Ok(())
}

fn listen_test(api: &Api, secs: u64, raw: bool) -> Result<()> {
    let tmp = std::env::temp_dir().join(format!("voxd_test_{}.wav", std::process::id()));
    println!("recording {secs}s from the default mic — speak now");
    let status = std::process::Command::new("ffmpeg")
        .args([
            "-y",
            "-hide_banner",
            "-loglevel",
            "error",
            "-f",
            "pulse",
            "-i",
            "default",
            "-ar",
            "16000",
            "-ac",
            "1",
            "-t",
            &secs.to_string(),
        ])
        .arg(&tmp)
        .status()
        .context("ffmpeg record")?;
    if !status.success() {
        bail!("ffmpeg record failed (is a microphone available?)");
    }
    let bytes = std::fs::read(&tmp).context("read recording")?;
    let _ = std::fs::remove_file(&tmp);
    let resp = api.post_raw("/listen/transcribe", bytes, "audio/wav")?;
    if raw {
        println!("{}", serde_json::to_string_pretty(&resp)?);
    } else {
        let t = resp["text"].as_str().unwrap_or("");
        if t.is_empty() {
            println!("(nothing heard)");
        } else {
            println!("heard: {t}");
        }
    }
    Ok(())
}

fn show_logs() {
    let path = voxd::config::default_state_dir().join("voxd.log");
    match std::fs::read_to_string(&path) {
        Ok(s) => {
            let lines: Vec<&str> = s.lines().collect();
            let start = lines.len().saturating_sub(50);
            for l in &lines[start..] {
                println!("{l}");
            }
        }
        Err(_) => println!("no logs at {}", path.display()),
    }
}
