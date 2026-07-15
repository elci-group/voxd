use anyhow::{bail, Context, Result};
use reqwest::multipart::{Form, Part};
use reqwest::Client;
use serde::Deserialize;
use serde_json::json;

use crate::{Settings, VoiceInfo};

const BASE: &str = "https://api.elevenlabs.io/v1";

#[derive(Clone)]
pub struct ElevenClient {
    http: Client,
    key: String,
    model: String,
    fmt: String,
}

impl ElevenClient {
    pub fn new(http: Client, key: String, model: String, fmt: String) -> Self {
        Self {
            http,
            key,
            model,
            fmt,
        }
    }

    pub fn model(&self) -> &str {
        &self.model
    }
    pub fn fmt(&self) -> &str {
        &self.fmt
    }

    /// Synthesize `text` with `voice_id`. Returns mp3 bytes.
    pub async fn speak(&self, text: &str, voice_id: &str, s: &Settings) -> Result<Vec<u8>> {
        let url = format!(
            "{BASE}/text-to-speech/{voice_id}?output_format={}",
            self.fmt
        );
        let body = json!({
            "text": text,
            "model_id": self.model,
            "voice_settings": {
                "stability": s.stability,
                "similarity_boost": s.similarity_boost,
                "style": s.style,
                "use_speaker_boost": s.use_speaker_boost,
            }
        });
        let resp = self
            .http
            .post(&url)
            .header("xi-api-key", &self.key)
            .header("Accept", "audio/mpeg")
            .json(&body)
            .send()
            .await
            .context("elevenlabs request")?;

        let status = resp.status();
        if !status.is_success() {
            let txt = resp.text().await.unwrap_or_default();
            bail!("elevenlabs HTTP {status}: {txt}");
        }
        let bytes = resp.bytes().await.context("read audio bytes")?;
        Ok(bytes.to_vec())
    }

    /// List available voices (premade + cloned) for the account.
    pub async fn list_voices(&self) -> Result<Vec<VoiceInfo>> {
        let url = format!("{BASE}/voices");
        let resp = self
            .http
            .get(&url)
            .header("xi-api-key", &self.key)
            .send()
            .await
            .context("list voices")?;
        let status = resp.status();
        if !status.is_success() {
            let txt = resp.text().await.unwrap_or_default();
            bail!("elevenlabs HTTP {status}: {txt}");
        }
        let parsed: VoicesResp = resp.json().await.context("parse voices")?;
        Ok(parsed.voices)
    }

    /// Transcribe audio bytes to text via ElevenLabs Scribe.
    pub async fn transcribe(&self, audio: Vec<u8>, mime: &str, stt_model: &str) -> Result<String> {
        let url = format!("{BASE}/speech-to-text");
        let file_name = if mime.contains("wav") {
            "audio.wav"
        } else {
            "audio.mp3"
        };
        let part = Part::bytes(audio)
            .file_name(file_name)
            .mime_str(mime)
            .context("mime")?;
        let form = Form::new()
            .part("file", part)
            .text("model_id", stt_model.to_string());
        let resp = self
            .http
            .post(&url)
            .header("xi-api-key", &self.key)
            .multipart(form)
            .send()
            .await
            .context("stt request")?;
        let status = resp.status();
        if !status.is_success() {
            let t = resp.text().await.unwrap_or_default();
            bail!("elevenlabs STT HTTP {status}: {t}");
        }
        let v: serde_json::Value = resp.json().await.context("parse stt")?;
        Ok(v.get("text")
            .and_then(|t| t.as_str())
            .unwrap_or("")
            .to_string())
    }

    /// Stream synthesized mp3 bytes (low time-to-first-audio). The caller is
    /// responsible for piping the stream to a player (see `audio::play_stream`).
    pub async fn speak_stream(
        &self,
        text: &str,
        voice_id: &str,
        s: &Settings,
    ) -> Result<impl futures_util::Stream<Item = Result<bytes::Bytes, reqwest::Error>>> {
        let url = format!(
            "{BASE}/text-to-speech/{voice_id}/stream?output_format={}&optimize_streaming_latency=3",
            self.fmt
        );
        let body = json!({
            "text": text,
            "model_id": self.model,
            "voice_settings": {
                "stability": s.stability,
                "similarity_boost": s.similarity_boost,
                "style": s.style,
                "use_speaker_boost": s.use_speaker_boost,
            }
        });
        let resp = self
            .http
            .post(&url)
            .header("xi-api-key", &self.key)
            .header("Accept", "audio/mpeg")
            .json(&body)
            .send()
            .await
            .context("tts stream request")?;
        let status = resp.status();
        if !status.is_success() {
            let t = resp.text().await.unwrap_or_default();
            bail!("elevenlabs stream HTTP {status}: {t}");
        }
        Ok(resp.bytes_stream())
    }
}

#[derive(Deserialize)]
struct VoicesResp {
    voices: Vec<VoiceInfo>,
}
