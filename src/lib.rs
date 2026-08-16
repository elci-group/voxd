//! voxd — local multi-provider speech daemon.
//!
//! Allocates and persists a distinct voice + personality per project, plus a
//! single unifying system voice for general / conversational responses.

pub mod alloc;
pub mod audio;
pub mod cache;
pub mod config;
pub mod elevenlabs;
pub mod groq;
pub mod listen;
pub mod mimic;
pub mod notify;
pub mod play;
pub mod project;
pub mod recap;
pub mod server;
pub mod state;
pub mod voices;

use serde::{Deserialize, Serialize};

/// TTS "personality" settings. ElevenLabs uses the voice-quality fields while
/// Groq Orpheus additionally sends `speed`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct Settings {
    pub stability: f32,
    pub similarity_boost: f32,
    pub style: f32,
    pub speed: f32,
    pub use_speaker_boost: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            stability: 0.5,
            similarity_boost: 0.75,
            style: 0.0,
            speed: 1.0,
            use_speaker_boost: true,
        }
    }
}

impl Settings {
    /// Merge: any field from `over` that is `Some` replaces the base value.
    pub fn apply(&self, over: &SettingsPatch) -> Self {
        Self {
            stability: over.stability.unwrap_or(self.stability),
            similarity_boost: over.similarity_boost.unwrap_or(self.similarity_boost),
            style: over.style.unwrap_or(self.style),
            speed: over.speed.unwrap_or(self.speed),
            use_speaker_boost: over.use_speaker_boost.unwrap_or(self.use_speaker_boost),
        }
    }

    /// Canonical provider-independent cache fragment. Some providers ignore a
    /// subset of fields, but every field that can affect synthesized audio is
    /// represented so cached speech is never reused with the wrong settings.
    pub fn cache_fragment(&self) -> String {
        format!(
            "{:.4}|{:.4}|{:.4}|{:.4}|{}",
            self.stability, self.similarity_boost, self.style, self.speed, self.use_speaker_boost
        )
    }
}

/// Partial settings override (e.g. from a CLI flag or API body).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SettingsPatch {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stability: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub similarity_boost: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub style: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speed: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub use_speaker_boost: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VoiceInfo {
    pub voice_id: String,
    pub name: String,
    #[serde(default)]
    pub category: String,
}

/// A persisted per-project voice/personality binding.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectRow {
    pub id: String,
    pub name: String,
    pub root_path: String,
    pub voice_id: String,
    pub label: String,
    #[serde(flatten)]
    pub settings: Settings,
    pub created_at: String,
    pub updated_at: String,
}

/// Resolved project identity (not yet persisted).
#[derive(Debug, Clone)]
pub struct ProjectRef {
    pub id: String,
    pub name: String,
    pub root_path: String,
}
