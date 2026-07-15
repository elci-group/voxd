//! Mic capture via an ffmpeg/parec child process producing 20 ms i16 frames.

use anyhow::{Context, Result};
use tokio::io::AsyncReadExt;
use tokio::process::{Child, Command};
use tokio::sync::mpsc;

/// Start capturing. Returns the child (so the caller can kill it on shutdown)
/// and a receiver of 20 ms frames (`Vec<i16>`, mono s16 at `sample_rate`).
pub async fn start(device: &str, sample_rate: u32) -> Result<(Child, mpsc::Receiver<Vec<i16>>)> {
    let samples_per_frame = (sample_rate / 50) as usize; // 20 ms
    let frame_bytes = samples_per_frame * 2;
    let (tx, rx) = mpsc::channel::<Vec<i16>>(64);

    let mut child = spawn_ffmpeg(device, sample_rate)
        .or_else(|e| {
            tracing::warn!(error = %e, "ffmpeg capture failed; trying parec");
            spawn_parec(device, sample_rate)
        })
        .context("no capture backend available (ffmpeg/parec)")?;

    let mut stdout = child.stdout.take().context("capture stdout")?;
    let dev = device.to_string();
    tokio::spawn(async move {
        let mut buf = vec![0u8; frame_bytes];
        while stdout.read_exact(&mut buf).await.is_ok() {
            let mut frame = Vec::with_capacity(samples_per_frame);
            for pair in buf.chunks_exact(2) {
                frame.push(i16::from_le_bytes([pair[0], pair[1]]));
            }
            if tx.send(frame).await.is_err() {
                break;
            }
        }
        tracing::debug!(device = %dev, "capture reader ended");
    });

    Ok((child, rx))
}

fn spawn_ffmpeg(device: &str, sample_rate: u32) -> Result<Child> {
    Command::new("ffmpeg")
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-f",
            "pulse",
            "-i",
            device,
            "-ar",
            &sample_rate.to_string(),
            "-ac",
            "1",
            "-f",
            "s16le",
            "-",
        ])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .context("spawn ffmpeg capture")
}

fn spawn_parec(device: &str, sample_rate: u32) -> Result<Child> {
    Command::new("parec")
        .args([
            "--format=s16le",
            &format!("--rate={sample_rate}"),
            "--channels=1",
            &format!("--device={device}"),
        ])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .context("spawn parec capture")
}
