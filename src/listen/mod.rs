//! Always-listening, keyword-activated speech-to-speech loop.

pub mod capture;
pub mod overlay;
pub mod responder;
pub mod vad;
pub mod wake;

use std::sync::Arc;

use anyhow::{Context, Result};
use tokio::sync::{oneshot, watch};

use crate::audio;
use crate::config::ListenCfg;
use crate::elevenlabs::ElevenClient;
use crate::play;
use crate::server::AppState;
use crate::Settings;

use self::capture::start as start_capture;
use self::overlay::OverlayHandle;
use self::responder::{IntentRouter, Responder};
use self::vad::Vad;
use self::wake::WakeMatcher;

/// Handle to a running listener. Dropping without calling `stop` detaches the
/// thread; prefer `stop` for a clean shutdown.
pub struct ListenHandle {
    stop: watch::Sender<bool>,
    join: std::thread::JoinHandle<()>,
}

impl ListenHandle {
    pub async fn stop(self) {
        let _ = self.stop.send(true);
        let _ = tokio::task::spawn_blocking(move || self.join.join()).await;
    }
}

/// Start the listener on its own single-threaded runtime, so all capture,
/// VAD, and STT work stays on one OS thread regardless of the server's
/// multi-threaded runtime. The caller owns the returned handle.
pub async fn start(state: Arc<AppState>) -> Result<ListenHandle> {
    let (stop_tx, stop_rx) = watch::channel(false);
    let (ready_tx, ready_rx) = oneshot::channel::<Result<()>>();
    let st = state.clone();
    let join = std::thread::Builder::new()
        .name("voxd-listen".into())
        .spawn(move || {
            let rt = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(rt) => rt,
                Err(e) => {
                    let _ = ready_tx.send(Err(anyhow::anyhow!(e).context("build listen runtime")));
                    return;
                }
            };
            rt.block_on(async move {
                if let Err(e) = run(st, stop_rx, ready_tx).await {
                    tracing::error!(error = %e, "listen loop stopped with error");
                }
            });
        })
        .context("spawn listen thread")?;
    match ready_rx.await {
        Ok(Ok(())) => Ok(ListenHandle {
            stop: stop_tx,
            join,
        }),
        Ok(Err(e)) => {
            let _ = stop_tx.send(true);
            let _ = join.join();
            Err(e)
        }
        Err(_) => Err(anyhow::anyhow!("listen thread died during startup")),
    }
}

async fn run(
    state: Arc<AppState>,
    mut stop_rx: watch::Receiver<bool>,
    ready_tx: oneshot::Sender<Result<()>>,
) -> Result<()> {
    let lc = state.cfg.listen.clone();
    let (mut child, mut frames) = match start_capture(&lc.device, lc.sample_rate).await {
        Ok(v) => {
            let _ = ready_tx.send(Ok(()));
            v
        }
        Err(e) => {
            let _ = ready_tx.send(Err(anyhow::anyhow!("{e:#}")));
            return Err(e);
        }
    };
    let mut overlay = OverlayHandle::spawn();
    overlay.send("listening");
    tracing::info!(wake = %lc.wake_word, device = %lc.device, overlay = overlay.is_active(), "listener started");

    let mut vad = Vad::new(
        lc.vad_threshold as f32,
        lc.silence_ms,
        lc.max_utterance_secs,
        lc.vad_noise_margin as f32,
    );
    let wake = WakeMatcher::new(&lc.wake_word);
    let responder = IntentRouter;
    let min_samples = (lc.min_utterance_ms * lc.sample_rate as u64 / 1000) as usize;

    loop {
        tokio::select! {
            _ = stop_rx.changed() => break,
            frame = frames.recv() => {
                match frame {
                    Some(f) => {
                        if let Some(utt) = vad.feed(&f) {
                            // Local gate: too short to contain the wake phrase
                            // (clicks, rustles) — drop without an STT call.
                            if utt.len() < min_samples {
                                tracing::debug!(samples = utt.len(), "skipped short utterance");
                                continue;
                            }
                            let stop = handle_utterance(&state, &lc, &wake, responder, utt)
                                .await
                                .unwrap_or_else(|e| {
                                    tracing::warn!(error = %e, "utterance handling failed");
                                    false
                                });
                            // Drop audio captured while we were speaking so it
                            // cannot re-trigger the wake matcher.
                            while frames.try_recv().is_ok() {}
                            vad.reset();
                            if stop { break; }
                        }
                    }
                    None => {
                        tracing::warn!("capture stream ended");
                        break;
                    }
                }
            }
        }
    }

    let _ = child.kill().await;
    tracing::info!("listener stopped");
    Ok(())
}

async fn handle_utterance(
    state: &AppState,
    lc: &ListenCfg,
    wake: &WakeMatcher,
    responder: IntentRouter,
    utt: Vec<i16>,
) -> Result<bool> {
    let client = state.eleven()?;
    let wav = audio::wav_from_pcm(&utt, lc.sample_rate);
    let text = client.transcribe(wav, "audio/wav", &lc.stt_model).await?;
    let text = text.trim().to_string();
    if text.is_empty() {
        return Ok(false);
    }
    tracing::info!(%text, "heard");

    let cmd = match wake.check(&text) {
        Some(c) => c,
        None => {
            tracing::debug!(%text, "ignored (no wake word)");
            return Ok(false);
        }
    };
    tracing::info!(%cmd, "command");

    let cmd_owned = cmd.clone();
    let reply = tokio::task::spawn_blocking(move || responder.respond(&cmd_owned))
        .await
        .context("responder join")?;

    let voice = if lc.reply_voice == "system" {
        state.cfg.system_voice.voice_id.clone()
    } else {
        lc.reply_voice.clone()
    };
    let settings = state.cfg.defaults;

    if state.cfg.mimic.enabled {
        match crate::server::synthesize_with_mimic(state, &reply.text, &voice, &settings).await {
            Ok(Some((bytes, _))) => {
                tokio::task::spawn_blocking(move || play::play_bytes_blocking(&bytes))
                    .await.context("play join")??;
            }
            Ok(None) => crate::notify::intended_message(&reply.text),
            Err(e) => {
                tracing::warn!(error = %e, "mimic preflight failed; using visual response");
                crate::notify::intended_message(&reply.text);
            }
        }
    } else if reply.low_latency && lc.low_latency {
        match client.speak_stream(&reply.text, &voice, &settings).await {
            Ok(stream) => {
                if let Err(e) = audio::play_stream(Box::pin(stream)).await {
                    tracing::warn!(error = %e, "streaming play failed");
                }
            }
            Err(e) => {
                tracing::warn!(error = %e, "streaming synth failed; falling back to batched");
                speak_batched(&client, &reply.text, &voice, &settings).await?;
            }
        }
    } else {
        speak_batched(&client, &reply.text, &voice, &settings).await?;
    }

    Ok(reply.stop)
}

async fn speak_batched(
    client: &ElevenClient,
    text: &str,
    voice: &str,
    settings: &Settings,
) -> Result<()> {
    let bytes = client.speak(text, voice, settings).await?;
    tokio::task::spawn_blocking(move || play::play_bytes_blocking(&bytes))
        .await
        .context("play join")??;
    Ok(())
}
