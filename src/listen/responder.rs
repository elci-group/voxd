//! Built-in intent router behind a pluggable `Responder` trait.

use std::process::Command;

#[derive(Debug, Clone)]
pub struct Reply {
    pub text: String,
    /// Use the low-latency streaming TTS path for short conversational replies.
    pub low_latency: bool,
    /// Stop the listener after speaking (e.g. "stop listening").
    pub stop: bool,
}

impl Reply {
    fn say(text: impl Into<String>) -> Self {
        let t = text.into();
        let low = t.len() <= 140;
        Self {
            text: t,
            low_latency: low,
            stop: false,
        }
    }
    fn stop(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            low_latency: true,
            stop: true,
        }
    }
}

pub trait Responder: Send + Sync {
    fn respond(&self, command: &str) -> Reply;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct IntentRouter;

impl Responder for IntentRouter {
    fn respond(&self, command: &str) -> Reply {
        let c = command.to_lowercase();
        let c = c.trim();
        if c.is_empty() {
            return Reply::say("Yes?");
        }
        if any_of(
            c,
            &[
                "stop listening",
                "go to sleep",
                "goodbye",
                "good bye",
                "shut down",
                "stop",
            ],
        ) {
            return Reply::stop("Stopping the listener. Goodbye.");
        }
        if c.contains("time") {
            return Reply::say(format!("It's {}.", chrono::Local::now().format("%H:%M")));
        }
        if c.contains("date") || c.contains("what day") || c.contains("today") {
            return Reply::say(format!(
                "Today is {}.",
                chrono::Local::now().format("%A, %B %-d")
            ));
        }
        if c.contains("uptime") {
            return Reply::say(
                shell_first("uptime", &["-p"]).unwrap_or_else(|| "Uptime unavailable.".into()),
            );
        }
        if c.contains("disk") || c.contains("storage") || c.contains("space") {
            return Reply::say(disk_summary());
        }
        if c.contains("spec") || c.contains("system") {
            return Reply::say(specs_summary());
        }
        if c.contains("status") {
            return Reply::say("voxd is running and listening.");
        }
        Reply::say(format!(
            "You said: {command}. I don't have an action for that yet."
        ))
    }
}

fn any_of(hay: &str, needles: &[&str]) -> bool {
    needles.iter().any(|n| hay.contains(n))
}

fn shell_first(bin: &str, args: &[&str]) -> Option<String> {
    let out = Command::new(bin).args(args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

fn disk_summary() -> String {
    let out = match Command::new("df").args(["-h", "/"]).output() {
        Ok(o) if o.status.success() => o.stdout,
        _ => return "Disk usage unavailable.".into(),
    };
    let s = String::from_utf8_lossy(&out);
    // Second line: Filesystem Size Used Avail Use% Mounted
    if let Some(line) = s.lines().nth(1) {
        let f: Vec<&str> = line.split_whitespace().collect();
        if f.len() >= 4 {
            return format!("Root disk: {} free of {} ({} used).", f[3], f[1], f[2]);
        }
    }
    "Disk usage unavailable.".into()
}

fn specs_summary() -> String {
    let cpu = Command::new("lscpu")
        .output()
        .ok()
        .and_then(|o| {
            let s = String::from_utf8_lossy(&o.stdout).to_string();
            s.lines()
                .find(|l| l.starts_with("Model name:"))
                .map(|l| l.trim_start_matches("Model name:").trim().to_string())
        })
        .unwrap_or_else(|| "unknown CPU".into());

    let mem = Command::new("free")
        .arg("-h")
        .output()
        .ok()
        .and_then(|o| {
            let s = String::from_utf8_lossy(&o.stdout).to_string();
            s.lines()
                .find(|l| l.starts_with("Mem:"))
                .and_then(|l| l.split_whitespace().nth(1).map(|t| t.to_string()))
        })
        .unwrap_or_else(|| "?".into());

    let disk_free = Command::new("df")
        .args(["-h", "/"])
        .output()
        .ok()
        .and_then(|o| {
            let s = String::from_utf8_lossy(&o.stdout).to_string();
            s.lines()
                .nth(1)
                .and_then(|l| l.split_whitespace().nth(3).map(|t| t.to_string()))
        })
        .unwrap_or_else(|| "?".into());

    Reply::say(format!("System: {cpu}, {mem} RAM, {disk_free} disk free.")).text
}
