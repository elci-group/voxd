use voxd::config::{Config, SpeechProvider};

#[test]
fn legacy_config_defaults_to_elevenlabs_providers() {
    let cfg: Config = toml::from_str("[server]\nbind = '127.0.0.1:17843'\n").unwrap();
    assert_eq!(cfg.providers.tts, SpeechProvider::Elevenlabs);
    assert_eq!(cfg.providers.stt, SpeechProvider::Elevenlabs);
    assert_eq!(cfg.groq.tts_model, "canopylabs/orpheus-v1-english");
    assert_eq!(cfg.groq.stt_model, "whisper-large-v3-turbo");
}

#[test]
fn groq_provider_and_settings_are_configurable() {
    let mut cfg = Config::default();
    cfg.set_key("providers.tts", "groq").unwrap();
    cfg.set_key("providers.stt", "groq").unwrap();
    cfg.set_key("groq.voice", "hannah").unwrap();
    cfg.set_key("groq.sample_rate", "24000").unwrap();

    assert_eq!(cfg.providers.tts, SpeechProvider::Groq);
    assert_eq!(cfg.providers.stt, SpeechProvider::Groq);
    assert_eq!(cfg.groq.voice, "hannah");
    assert_eq!(cfg.groq.sample_rate, 24_000);
    assert!(cfg.set_key("groq.output_format", "mp3").is_err());
    assert!(cfg.set_key("providers.tts", "unknown").is_err());
}

#[test]
fn groq_voice_catalog_tracks_model_language() {
    let english = voxd::groq::voices("canopylabs/orpheus-v1-english");
    let arabic = voxd::groq::voices("canopylabs/orpheus-arabic-saudi");
    assert!(english.iter().any(|voice| voice.voice_id == "troy"));
    assert!(!english.iter().any(|voice| voice.voice_id == "fahad"));
    assert!(arabic.iter().any(|voice| voice.voice_id == "fahad"));
}
