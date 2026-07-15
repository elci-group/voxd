//! Built-in fallback voice pool.
//!
//! Some ElevenLabs API keys are restricted and lack the `voices_read`
//! permission needed for `GET /v1/voices`. To keep per-project allocation
//! working with such keys, we ship a static pool of well-known, stable premade
//! voice ids. These are the canonical defaults published by ElevenLabs; they
//! are used only when the live voice list cannot be fetched and the user has
//! not configured an explicit `[pool].voices`.

use crate::VoiceInfo;

/// (voice_id, display name) for canonical ElevenLabs premade voices.
pub const BUILTIN: &[(&str, &str)] = &[
    ("21m00Tcm4TlvDq8ikWAM", "Rachel"),
    ("AZnzlk1XvdvUeBnXmlld", "Domi"),
    ("EXAVITQu4vr4xnSDxMaL", "Bella"),
    ("ErXwobaYiN019PkySvjV", "Antoni"),
    ("MF3mGyEYCl7XYWbV9V6O", "Elli"),
    ("TxGEqnHWrfWFTfGW9XjX", "Josh"),
    ("VR6AewLTigWG4xSOukaG", "Arnold"),
    ("pNInz6obpgDQGcFmaJgB", "Adam"),
    ("yoZ06aMxZJJ28mfd3POQ", "Sam"),
    ("jBpfuIE2acCO8z3wKNLl", "Gigi"),
    ("jsCqWAovK2LkecY7zXl4", "Freya"),
    ("oWAxZDx7w5VEj9dCyTzz", "Grace"),
    ("piTKgcLEGmPE4e6mEKli", "Nicole"),
    ("t0jbNlBVZ17f02VDIeMI", "Jessie"),
    ("z9fAnlkpzviPz146aGWa", "Glinda"),
];

/// Built-in pool as `VoiceInfo` entries (category "premade").
pub fn builtin_voices() -> Vec<VoiceInfo> {
    BUILTIN
        .iter()
        .map(|(id, name)| VoiceInfo {
            voice_id: (*id).to_string(),
            name: (*name).to_string(),
            category: "premade".to_string(),
        })
        .collect()
}

/// Built-in pool ids, optionally excluding one (e.g. the system voice).
pub fn builtin_ids_excluding(exclude: &str) -> Vec<String> {
    BUILTIN
        .iter()
        .map(|(id, _)| (*id).to_string())
        .filter(|id| id != exclude)
        .collect()
}
