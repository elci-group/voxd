use anyhow::{bail, Context, Result};
use reqwest::multipart::{Form, Part};
use reqwest::Client;
use serde_json::json;

use crate::{Settings, VoiceInfo};

const BASE: &str = "https://api.groq.com/openai/v1";
const MAX_TTS_CHARS: usize = 200;

const ENGLISH_VOICES: &[(&str, &str)] = &[
    ("autumn", "Autumn"),
    ("diana", "Diana"),
    ("hannah", "Hannah"),
    ("austin", "Austin"),
    ("daniel", "Daniel"),
    ("troy", "Troy"),
];

const ARABIC_VOICES: &[(&str, &str)] = &[
    ("abdullah", "Abdullah"),
    ("fahad", "Fahad"),
    ("sultan", "Sultan"),
    ("lulwa", "Lulwa"),
    ("noura", "Noura"),
    ("aisha", "Aisha"),
];

#[derive(Clone)]
pub struct GroqClient {
    http: Client,
    key: String,
    base: String,
    tts_model: String,
    output_format: String,
    sample_rate: u32,
    stt_model: String,
}

impl GroqClient {
    pub fn new(
        http: Client,
        key: String,
        tts_model: String,
        output_format: String,
        sample_rate: u32,
        stt_model: String,
    ) -> Self {
        Self {
            http,
            key,
            base: BASE.into(),
            tts_model,
            output_format,
            sample_rate,
            stt_model,
        }
    }

    pub fn tts_model(&self) -> &str {
        &self.tts_model
    }

    pub fn output_format(&self) -> &str {
        &self.output_format
    }

    /// Synthesize speech with Groq Orpheus. Orpheus accepts at most 200
    /// characters, so longer text is split at natural boundaries and the
    /// resulting PCM WAV files are joined losslessly.
    pub async fn speak(&self, text: &str, voice: &str, settings: &Settings) -> Result<Vec<u8>> {
        if self.output_format != "wav" {
            bail!("Groq Orpheus currently requires output_format = 'wav'");
        }
        let chunks = split_for_tts(text, MAX_TTS_CHARS);
        if chunks.is_empty() {
            bail!("empty text");
        }

        let mut audio = Vec::with_capacity(chunks.len());
        for chunk in chunks {
            let body = json!({
                "model": self.tts_model,
                "input": chunk,
                "voice": voice,
                "response_format": self.output_format,
                "sample_rate": self.sample_rate,
                "speed": settings.speed.clamp(0.5, 5.0),
            });
            let resp = self
                .http
                .post(format!("{}/audio/speech", self.base))
                .bearer_auth(&self.key)
                .header("Accept", "audio/wav")
                .json(&body)
                .send()
                .await
                .context("Groq TTS request")?;
            let status = resp.status();
            if !status.is_success() {
                let text = resp.text().await.unwrap_or_default();
                bail!("Groq TTS HTTP {status}: {text}");
            }
            audio.push(resp.bytes().await.context("read Groq TTS audio")?.to_vec());
        }
        join_wav(&audio)
    }

    pub async fn transcribe(&self, audio: Vec<u8>, mime: &str) -> Result<String> {
        let file_name = file_name_for_mime(mime);
        let part = Part::bytes(audio)
            .file_name(file_name)
            .mime_str(mime)
            .context("audio MIME type")?;
        let form = Form::new()
            .part("file", part)
            .text("model", self.stt_model.clone())
            .text("response_format", "json");
        let resp = self
            .http
            .post(format!("{}/audio/transcriptions", self.base))
            .bearer_auth(&self.key)
            .multipart(form)
            .send()
            .await
            .context("Groq STT request")?;
        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            bail!("Groq STT HTTP {status}: {text}");
        }
        let value: serde_json::Value = resp.json().await.context("parse Groq STT response")?;
        Ok(value
            .get("text")
            .and_then(|text| text.as_str())
            .unwrap_or_default()
            .to_string())
    }
}

pub fn voices(model: &str) -> Vec<VoiceInfo> {
    let source = if model.contains("arabic") {
        ARABIC_VOICES
    } else {
        ENGLISH_VOICES
    };
    source
        .iter()
        .map(|(voice_id, name)| VoiceInfo {
            voice_id: (*voice_id).into(),
            name: (*name).into(),
            category: "groq/orpheus".into(),
        })
        .collect()
}

pub fn supports_voice(model: &str, voice: &str) -> bool {
    voices(model).iter().any(|item| item.voice_id == voice)
}

fn file_name_for_mime(mime: &str) -> &'static str {
    if mime.contains("mpeg") || mime.contains("mp3") {
        "audio.mp3"
    } else if mime.contains("flac") {
        "audio.flac"
    } else if mime.contains("ogg") {
        "audio.ogg"
    } else if mime.contains("webm") {
        "audio.webm"
    } else {
        "audio.wav"
    }
}

fn split_for_tts(text: &str, max_chars: usize) -> Vec<String> {
    let mut remaining = text.trim();
    let mut chunks = Vec::new();
    while remaining.chars().count() > max_chars {
        let byte_limit = remaining
            .char_indices()
            .nth(max_chars)
            .map(|(index, _)| index)
            .unwrap_or(remaining.len());
        let window = &remaining[..byte_limit];
        let min_break = max_chars / 2;
        let split = window
            .char_indices()
            .rev()
            .find(|(index, ch)| {
                window[..*index].chars().count() >= min_break
                    && (ch.is_whitespace() || matches!(ch, '.' | '!' | '?' | ';' | ':' | ','))
            })
            .map(|(index, ch)| {
                if ch.is_whitespace() {
                    index
                } else {
                    index + ch.len_utf8()
                }
            })
            .unwrap_or(byte_limit);
        chunks.push(remaining[..split].trim().to_string());
        remaining = remaining[split..].trim_start();
    }
    if !remaining.is_empty() {
        chunks.push(remaining.to_string());
    }
    chunks
}

fn join_wav(parts: &[Vec<u8>]) -> Result<Vec<u8>> {
    match parts {
        [] => bail!("Groq returned no audio"),
        [only] => return Ok(only.clone()),
        _ => {}
    }

    let mut format: Option<Vec<u8>> = None;
    let mut data = Vec::new();
    for wav in parts {
        let (part_format, part_data) = wav_chunks(wav)?;
        if let Some(expected) = &format {
            if expected != part_format {
                bail!("Groq returned incompatible WAV formats across TTS chunks");
            }
        } else {
            format = Some(part_format.to_vec());
        }
        data.extend_from_slice(part_data);
    }

    let format = format.context("WAV format chunk")?;
    let fmt_pad = format.len() % 2;
    let data_pad = data.len() % 2;
    let total_len = 12 + 8 + format.len() + fmt_pad + 8 + data.len() + data_pad;
    let mut out = Vec::with_capacity(total_len);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&((total_len - 8) as u32).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&(format.len() as u32).to_le_bytes());
    out.extend_from_slice(&format);
    if fmt_pad != 0 {
        out.push(0);
    }
    out.extend_from_slice(b"data");
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    out.extend_from_slice(&data);
    if data_pad != 0 {
        out.push(0);
    }
    Ok(out)
}

fn wav_chunks(wav: &[u8]) -> Result<(&[u8], &[u8])> {
    if wav.len() < 12 || &wav[..4] != b"RIFF" || &wav[8..12] != b"WAVE" {
        bail!("Groq returned invalid WAV audio");
    }
    let mut cursor = 12usize;
    let mut format = None;
    let mut data = None;
    while cursor + 8 <= wav.len() {
        let id = &wav[cursor..cursor + 4];
        let size = u32::from_le_bytes(wav[cursor + 4..cursor + 8].try_into().unwrap()) as usize;
        let start = cursor + 8;
        let end = start
            .checked_add(size)
            .filter(|end| *end <= wav.len())
            .context("truncated WAV chunk")?;
        if id == b"fmt " {
            format = Some(&wav[start..end]);
        } else if id == b"data" {
            data = Some(&wav[start..end]);
        }
        cursor = end + (size % 2);
    }
    Ok((
        format.context("missing WAV format chunk")?,
        data.context("missing WAV data chunk")?,
    ))
}

#[cfg(test)]
mod tests {
    use super::{join_wav, split_for_tts};
    use crate::audio::wav_from_pcm;

    #[test]
    fn chunks_unicode_text_at_or_below_provider_limit() {
        let text = format!("{} {}", "hello ".repeat(40), "🦀".repeat(90));
        let chunks = split_for_tts(&text, 200);
        assert!(chunks.len() >= 2);
        assert!(chunks.iter().all(|chunk| chunk.chars().count() <= 200));
        assert!(chunks.iter().all(|chunk| !chunk.is_empty()));
    }

    #[test]
    fn joins_pcm_wav_data() {
        let first = wav_from_pcm(&[1, 2], 16_000);
        let second = wav_from_pcm(&[3, 4, 5], 16_000);
        let joined = join_wav(&[first, second]).unwrap();
        assert_eq!(&joined[..4], b"RIFF");
        assert_eq!(u32::from_le_bytes(joined[40..44].try_into().unwrap()), 10);
        assert_eq!(joined.len(), 54);
    }
}
