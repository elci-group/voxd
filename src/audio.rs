use anyhow::{Context, Result};
use bytes::Bytes;
use futures_util::{Stream, StreamExt};
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

/// Encode signed 16-bit little-endian mono PCM as a WAV byte buffer.
pub fn wav_from_pcm(samples: &[i16], sample_rate: u32) -> Vec<u8> {
    let channels: u16 = 1;
    let bits: u16 = 16;
    let byte_rate = sample_rate * channels as u32 * (bits as u32 / 8);
    let block_align: u16 = channels * (bits / 8);
    let data_len = (samples.len() * 2) as u32;
    let riff_len = 36 + data_len;

    let mut out = Vec::with_capacity(44 + data_len as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&riff_len.to_le_bytes());
    out.extend_from_slice(b"WAVE");
    out.extend_from_slice(b"fmt ");
    out.extend_from_slice(&16u32.to_le_bytes()); // fmt chunk size
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&channels.to_le_bytes());
    out.extend_from_slice(&sample_rate.to_le_bytes());
    out.extend_from_slice(&byte_rate.to_le_bytes());
    out.extend_from_slice(&block_align.to_le_bytes());
    out.extend_from_slice(&bits.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for s in samples {
        out.extend_from_slice(&s.to_le_bytes());
    }
    out
}

/// Stream an mp3 byte stream into ffplay for low time-to-first-audio playback.
/// Falls back to draining the stream if ffplay cannot be spawned.
pub async fn play_stream<S>(stream: S) -> Result<()>
where
    S: Stream<Item = Result<Bytes, reqwest::Error>> + Send + Unpin + 'static,
{
    let mut child = match Command::new("ffplay")
        .args([
            "-nodisp",
            "-autoexit",
            "-loglevel",
            "quiet",
            "-f",
            "mp3",
            "-i",
            "pipe:0",
        ])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            // No player: drain the stream so the request completes cleanly.
            tracing::warn!(error = %e, "ffplay unavailable for streaming; draining");
            let mut s = stream;
            while let Some(_c) = s.next().await {}
            return Ok(());
        }
    };

    let stdin = child.stdin.take().context("ffplay stdin")?;
    let writer = tokio::spawn(pipe_to_stdin(stream, stdin));
    let _ = child.wait().await;
    writer.await.context("join ffplay writer")??;
    Ok(())
}

async fn pipe_to_stdin<S, W>(mut stream: S, mut stdin: W) -> Result<()>
where
    S: Stream<Item = Result<Bytes, reqwest::Error>> + Send + Unpin,
    W: tokio::io::AsyncWrite + Unpin,
{
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        if stdin.write_all(&chunk).await.is_err() {
            break; // player closed early
        }
    }
    Ok(())
}
