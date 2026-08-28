use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use rand::RngCore;
use serde::{Deserialize, Serialize};

use crate::{Settings, SettingsPatch};

const DEFAULT_BIND: &str = "127.0.0.1:17843";
const DEFAULT_SYSTEM_VOICE: &str = "21m00Tcm4TlvDq8ikWAM"; // Rachel (premade)

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Config {
    #[serde(default)]
    pub server: ServerCfg,
    #[serde(default)]
    pub elevenlabs: ElevenCfg,
    #[serde(default)]
    pub groq: GroqCfg,
    #[serde(default)]
    pub providers: ProviderCfg,
    #[serde(default)]
    pub system_voice: SystemVoice,
    #[serde(default)]
    pub defaults: Settings,
    #[serde(default)]
    pub pool: PoolCfg,
    #[serde(default)]
    pub cache: CacheCfg,
    #[serde(default)]
    pub listen: ListenCfg,
    #[serde(default)]
    pub mimic: MimicCfg,
    #[serde(default)]
    pub recap: RecapCfg,
    #[serde(default)]
    pub voices: HashMap<String, VoiceProfileCfg>,
    #[serde(default)]
    pub routing: RoutingCfg,
    #[serde(default)]
    pub hotkey: HotkeyCfg,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum SpeechProvider {
    #[default]
    Elevenlabs,
    Groq,
}

impl std::str::FromStr for SpeechProvider {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "elevenlabs" | "eleven" => Ok(Self::Elevenlabs),
            "groq" => Ok(Self::Groq),
            _ => bail!("speech provider must be 'elevenlabs' or 'groq'"),
        }
    }
}

impl std::fmt::Display for SpeechProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Elevenlabs => f.write_str("elevenlabs"),
            Self::Groq => f.write_str("groq"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderCfg {
    #[serde(default)]
    pub tts: SpeechProvider,
    #[serde(default)]
    pub stt: SpeechProvider,
}

impl Default for ProviderCfg {
    fn default() -> Self {
        Self {
            tts: SpeechProvider::Elevenlabs,
            stt: SpeechProvider::Elevenlabs,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MimicCfg {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "d_mimic_url")]
    pub url: String,
    #[serde(default)]
    pub auth_token: String,
    #[serde(default = "d_pv_bin")]
    pub pv_bin: String,
    #[serde(default = "d_mimic_objects")]
    pub object_root: String,
    /// Record per-synthesis efficiency metrics in `state.db` and emit
    /// structured `voxd::mimic::telemetry` tracing events.
    #[serde(default = "d_true")]
    pub telemetry_enabled: bool,
}

impl Default for MimicCfg {
    fn default() -> Self {
        Self {
            enabled: false,
            url: d_mimic_url(),
            auth_token: String::new(),
            pv_bin: d_pv_bin(),
            object_root: d_mimic_objects(),
            telemetry_enabled: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecapCfg {
    /// Watch supported harnesses' session logs and speak recaps automatically.
    #[serde(default = "d_true")]
    pub enabled: bool,
    #[serde(default = "d_recap_poll")]
    pub poll_interval_secs: u64,
    /// Claude Code session-log root; auto-detected if it does not exist.
    #[serde(default = "d_claude_projects_dir")]
    pub claude_projects_dir: String,
}
impl Default for RecapCfg {
    fn default() -> Self {
        Self {
            enabled: true,
            poll_interval_secs: d_recap_poll(),
            claude_projects_dir: d_claude_projects_dir(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerCfg {
    #[serde(default = "d_bind")]
    pub bind: String,
    #[serde(default)]
    pub auth_token: String,
    #[serde(default = "d_pid")]
    pub pid_file: String,
}
impl Default for ServerCfg {
    fn default() -> Self {
        Self {
            bind: d_bind(),
            auth_token: String::new(),
            pid_file: d_pid(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ElevenCfg {
    #[serde(default = "d_model")]
    pub model_id: String,
    #[serde(default = "d_fmt")]
    pub output_format: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
}
impl Default for ElevenCfg {
    fn default() -> Self {
        Self {
            model_id: d_model(),
            output_format: d_fmt(),
            api_key: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroqCfg {
    #[serde(default = "d_groq_tts_model")]
    pub tts_model: String,
    #[serde(default = "d_groq_voice")]
    pub voice: String,
    #[serde(default = "d_groq_fmt")]
    pub output_format: String,
    #[serde(default = "d_groq_sample_rate")]
    pub sample_rate: u32,
    #[serde(default = "d_groq_stt_model")]
    pub stt_model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
}

impl Default for GroqCfg {
    fn default() -> Self {
        Self {
            tts_model: d_groq_tts_model(),
            voice: d_groq_voice(),
            output_format: d_groq_fmt(),
            sample_rate: d_groq_sample_rate(),
            stt_model: d_groq_stt_model(),
            api_key: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemVoice {
    #[serde(default = "d_sysvoice")]
    pub voice_id: String,
    #[serde(default = "d_syslabel")]
    pub label: String,
}
impl Default for SystemVoice {
    fn default() -> Self {
        Self {
            voice_id: d_sysvoice(),
            label: d_syslabel(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PoolCfg {
    #[serde(default)]
    pub voices: Vec<String>,
}

/// A named voice profile decouples voice identity from the synthesis provider.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct VoiceProfileCfg {
    /// Optional provider override. When omitted the active `providers.tts` is used.
    #[serde(default)]
    pub provider: Option<SpeechProvider>,
    /// Concrete voice id for the provider.
    pub voice_id: String,
    /// Human-readable label for this profile.
    #[serde(default)]
    pub label: String,
    /// Optional per-profile settings override.
    #[serde(flatten)]
    pub settings: SettingsPatch,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum RuleScope {
    Default,
    Application,
    Domain,
    Window,
    Explicit,
}

impl std::str::FromStr for RuleScope {
    type Err = anyhow::Error;
    fn from_str(value: &str) -> Result<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "default" => Ok(Self::Default),
            "application" | "app" => Ok(Self::Application),
            "domain" => Ok(Self::Domain),
            "window" => Ok(Self::Window),
            "explicit" => Ok(Self::Explicit),
            _ => bail!("rule scope must be default, application, domain, window, or explicit"),
        }
    }
}

impl std::fmt::Display for RuleScope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Default => f.write_str("default"),
            Self::Application => f.write_str("application"),
            Self::Domain => f.write_str("domain"),
            Self::Window => f.write_str("window"),
            Self::Explicit => f.write_str("explicit"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoutingRuleCfg {
    pub scope: RuleScope,
    pub pattern: String,
    pub voice: String,
    #[serde(default)]
    pub priority: i32,
    #[serde(default = "d_true")]
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoutingCfg {
    #[serde(default = "d_routing_default_voice")]
    pub default_voice: String,
    #[serde(default)]
    pub rules: Vec<RoutingRuleCfg>,
}

impl Default for RoutingCfg {
    fn default() -> Self {
        Self {
            default_voice: d_routing_default_voice(),
            rules: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HotkeyCfg {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "d_hotkey_device")]
    pub device: String,
    #[serde(default = "d_hotkey_speak")]
    pub speak_selection: String,
    #[serde(default = "d_hotkey_stop")]
    pub stop: String,
}

impl Default for HotkeyCfg {
    fn default() -> Self {
        Self {
            enabled: false,
            device: d_hotkey_device(),
            speak_selection: d_hotkey_speak(),
            stop: d_hotkey_stop(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheCfg {
    #[serde(default = "d_cachedir")]
    pub dir: String,
    #[serde(default = "d_true")]
    pub enabled: bool,
    #[serde(default = "d_maxmb")]
    pub max_mb: u64,
}
impl Default for CacheCfg {
    fn default() -> Self {
        Self {
            dir: d_cachedir(),
            enabled: true,
            max_mb: d_maxmb(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListenCfg {
    #[serde(default = "d_wake")]
    pub wake_word: String,
    #[serde(default = "d_device")]
    pub device: String,
    #[serde(default = "d_sr")]
    pub sample_rate: u32,
    #[serde(default = "d_vad")]
    pub vad_threshold: f64,
    /// Speech must exceed `noise_floor * vad_noise_margin` to trigger; the
    /// floor adapts to sustained ambient noise so it stops hitting STT.
    #[serde(default = "d_vadmargin")]
    pub vad_noise_margin: f64,
    /// Utterances shorter than this are dropped locally, no STT call.
    #[serde(default = "d_minutt")]
    pub min_utterance_ms: u64,
    #[serde(default = "d_sil")]
    pub silence_ms: u64,
    #[serde(default = "d_maxutt")]
    pub max_utterance_secs: u64,
    #[serde(default = "d_true")]
    pub low_latency: bool,
    #[serde(default = "d_stt")]
    pub stt_model: String,
    #[serde(default = "d_replyvoice")]
    pub reply_voice: String,
}
impl Default for ListenCfg {
    fn default() -> Self {
        Self {
            wake_word: d_wake(),
            device: d_device(),
            sample_rate: d_sr(),
            vad_threshold: d_vad(),
            vad_noise_margin: d_vadmargin(),
            min_utterance_ms: d_minutt(),
            silence_ms: d_sil(),
            max_utterance_secs: d_maxutt(),
            low_latency: true,
            stt_model: d_stt(),
            reply_voice: d_replyvoice(),
        }
    }
}

fn d_bind() -> String {
    DEFAULT_BIND.into()
}
fn d_pid() -> String {
    default_state_dir().join("voxd.pid").display().to_string()
}
fn d_model() -> String {
    "eleven_multilingual_v2".into()
}
fn d_fmt() -> String {
    "mp3_44100_128".into()
}
fn d_groq_tts_model() -> String {
    "canopylabs/orpheus-v1-english".into()
}
fn d_groq_voice() -> String {
    "troy".into()
}
fn d_groq_fmt() -> String {
    "wav".into()
}
fn d_groq_sample_rate() -> u32 {
    48000
}
fn d_groq_stt_model() -> String {
    "whisper-large-v3-turbo".into()
}
fn d_sysvoice() -> String {
    DEFAULT_SYSTEM_VOICE.into()
}
fn d_syslabel() -> String {
    "system".into()
}
fn d_true() -> bool {
    true
}
fn d_maxmb() -> u64 {
    512
}
fn d_cachedir() -> String {
    default_cache_dir().display().to_string()
}
fn d_wake() -> String {
    "hey voxd".into()
}
fn d_device() -> String {
    "default".into()
}
fn d_sr() -> u32 {
    16000
}
fn d_vad() -> f64 {
    0.02
}
fn d_vadmargin() -> f64 {
    3.0
}
fn d_minutt() -> u64 {
    400
}
fn d_sil() -> u64 {
    700
}
fn d_maxutt() -> u64 {
    12
}
fn d_stt() -> String {
    "scribe_v1".into()
}
fn d_replyvoice() -> String {
    "system".into()
}
fn d_mimic_url() -> String {
    "http://127.0.0.1:17844".into()
}
fn d_pv_bin() -> String {
    "pv".into()
}
fn d_mimic_objects() -> String {
    "~/.local/share/mimic/objects".into()
}
fn d_recap_poll() -> u64 {
    2
}
fn d_claude_projects_dir() -> String {
    "~/.claude/projects".into()
}
fn d_routing_default_voice() -> String {
    "system".into()
}
fn d_hotkey_device() -> String {
    "auto".into()
}
fn d_hotkey_speak() -> String {
    "LEFTMETA+LEFTSHIFT+V".into()
}
fn d_hotkey_stop() -> String {
    "LEFTMETA+LEFTSHIFT+S".into()
}

// ---- path helpers ---------------------------------------------------------

fn home() -> PathBuf {
    PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| ".".into()))
}

pub fn config_dir() -> PathBuf {
    if let Ok(x) = std::env::var("XDG_CONFIG_HOME") {
        return PathBuf::from(x).join("voxd");
    }
    home().join(".config").join("voxd")
}

pub fn default_config_path() -> PathBuf {
    config_dir().join("config.toml")
}

pub fn default_state_dir() -> PathBuf {
    if let Ok(x) = std::env::var("XDG_STATE_HOME") {
        return PathBuf::from(x).join("voxd");
    }
    home().join(".local").join("share").join("voxd")
}

pub fn default_cache_dir() -> PathBuf {
    if let Ok(x) = std::env::var("XDG_CACHE_HOME") {
        return PathBuf::from(x).join("voxd");
    }
    home().join(".cache").join("voxd")
}

pub fn expand_tilde(p: &str) -> PathBuf {
    if let Some(rest) = p.strip_prefix("~/") {
        return home().join(rest);
    }
    PathBuf::from(p)
}

impl Config {
    pub fn cache_dir(&self) -> PathBuf {
        expand_tilde(&self.cache.dir)
    }
    pub fn pid_file(&self) -> PathBuf {
        expand_tilde(&self.server.pid_file)
    }
    pub fn state_db(&self) -> PathBuf {
        default_state_dir().join("state.db")
    }
    pub fn log_file(&self) -> PathBuf {
        default_state_dir().join("voxd.log")
    }

    pub fn set_key(&mut self, key: &str, value: &str) -> Result<()> {
        match key {
            "server.bind" => self.server.bind = value.to_string(),
            "server.pid_file" => self.server.pid_file = value.to_string(),
            "elevenlabs.model_id" => self.elevenlabs.model_id = value.to_string(),
            "elevenlabs.output_format" => self.elevenlabs.output_format = value.to_string(),
            "elevenlabs.api_key" => {
                self.elevenlabs.api_key = if value.trim().is_empty() {
                    None
                } else {
                    Some(value.to_string())
                };
            }
            "groq.tts_model" => self.groq.tts_model = value.to_string(),
            "groq.voice" => self.groq.voice = value.to_string(),
            "groq.output_format" => {
                if value != "wav" {
                    bail!("groq.output_format must be 'wav'");
                }
                self.groq.output_format = value.to_string();
            }
            "groq.sample_rate" => self.groq.sample_rate = parse_groq_sample_rate(key, value)?,
            "groq.stt_model" => self.groq.stt_model = value.to_string(),
            "groq.api_key" => {
                self.groq.api_key = if value.trim().is_empty() {
                    None
                } else {
                    Some(value.to_string())
                };
            }
            "providers.tts" => self.providers.tts = value.parse()?,
            "providers.stt" => self.providers.stt = value.parse()?,
            "system_voice.voice_id" => self.system_voice.voice_id = value.to_string(),
            "system_voice.label" => self.system_voice.label = value.to_string(),
            "defaults.stability" => self.defaults.stability = parse_f32(key, value)?,
            "defaults.similarity_boost" => self.defaults.similarity_boost = parse_f32(key, value)?,
            "defaults.style" => self.defaults.style = parse_f32(key, value)?,
            "defaults.speed" => self.defaults.speed = parse_f32(key, value)?,
            "defaults.use_speaker_boost" => {
                self.defaults.use_speaker_boost = parse_bool(key, value)?
            }
            "cache.dir" => self.cache.dir = value.to_string(),
            "cache.enabled" => self.cache.enabled = parse_bool(key, value)?,
            "cache.max_mb" => self.cache.max_mb = parse_u64(key, value)?,
            "listen.wake_word" => self.listen.wake_word = value.to_string(),
            "listen.device" => self.listen.device = value.to_string(),
            "listen.sample_rate" => self.listen.sample_rate = parse_u32(key, value)?,
            "listen.vad_threshold" => self.listen.vad_threshold = parse_f64(key, value)?,
            "listen.vad_noise_margin" => self.listen.vad_noise_margin = parse_f64(key, value)?,
            "listen.min_utterance_ms" => self.listen.min_utterance_ms = parse_u64(key, value)?,
            "listen.silence_ms" => self.listen.silence_ms = parse_u64(key, value)?,
            "listen.max_utterance_secs" => self.listen.max_utterance_secs = parse_u64(key, value)?,
            "listen.low_latency" => self.listen.low_latency = parse_bool(key, value)?,
            "listen.stt_model" => self.listen.stt_model = value.to_string(),
            "listen.reply_voice" => self.listen.reply_voice = value.to_string(),
            "mimic.enabled" => self.mimic.enabled = parse_bool(key, value)?,
            "mimic.url" => self.mimic.url = value.to_string(),
            "mimic.auth_token" => self.mimic.auth_token = value.to_string(),
            "mimic.pv_bin" => self.mimic.pv_bin = value.to_string(),
            "mimic.object_root" => self.mimic.object_root = value.to_string(),
            "mimic.telemetry_enabled" => self.mimic.telemetry_enabled = parse_bool(key, value)?,
            "recap.enabled" => self.recap.enabled = parse_bool(key, value)?,
            "recap.poll_interval_secs" => self.recap.poll_interval_secs = parse_u64(key, value)?,
            "recap.claude_projects_dir" => self.recap.claude_projects_dir = value.to_string(),
            "routing.default_voice" => self.routing.default_voice = value.to_string(),
            "hotkey.enabled" => self.hotkey.enabled = parse_bool(key, value)?,
            "hotkey.device" => self.hotkey.device = value.to_string(),
            "hotkey.speak_selection" => self.hotkey.speak_selection = value.to_string(),
            "hotkey.stop" => self.hotkey.stop = value.to_string(),
            _ => {
                // voices.<name>.voice_id / provider / label / settings.*
                if let Some(rest) = key.strip_prefix("voices.") {
                    let mut parts = rest.splitn(3, '.');
                    let name = parts.next().ok_or_else(|| anyhow!("invalid voice key {key}"))?;
                    let sub = parts.next().ok_or_else(|| anyhow!("invalid voice key {key}"))?;
                    let profile = self.voices.entry(name.to_string()).or_default();
                    match sub {
                        "voice_id" => profile.voice_id = value.to_string(),
                        "provider" => {
                            profile.provider = if value.trim().is_empty() {
                                None
                            } else {
                                Some(value.parse()?)
                            }
                        }
                        "label" => profile.label = value.to_string(),
                        "settings" => {
                            let field = parts.next().ok_or_else(|| anyhow!("invalid voice settings key {key}"))?;
                            apply_voice_setting(&mut profile.settings, field, value)?;
                        }
                        _ => bail!("unknown voice field {sub}"),
                    }
                    return Ok(());
                }
                bail!("unknown config key {key}")
            }
        }
        Ok(())
    }
}

/// Load config from `path`, creating a default file (with a fresh auth token)
/// when it does not exist. An empty auth_token is regenerated and persisted.
pub fn load_or_init(path: &Path) -> Result<Config> {
    if !path.exists() {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
        }
        let mut cfg = Config::default();
        cfg.server.auth_token = gen_token();
        save_config(path, &cfg)?;
        return Ok(cfg);
    }
    let raw = fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    let mut cfg: Config =
        toml::from_str(&raw).with_context(|| format!("parse {}", path.display()))?;
    let mut wrote = false;
    if cfg.server.auth_token.is_empty() {
        cfg.server.auth_token = gen_token();
        save_config(path, &cfg)?;
        wrote = true;
    }
    // Backfill newly-added sections so the on-disk file reflects current
    // defaults and remains user-tunable.
    if ["[listen]", "[groq]", "[providers]", "[recap]", "[routing]", "[hotkey]", "[mimic]"]
        .iter()
        .any(|section| !raw.contains(section))
        && !wrote
    {
        save_config(path, &cfg)?;
    }
    Ok(cfg)
}

pub fn save_config(path: &Path, cfg: &Config) -> Result<()> {
    let s = toml::to_string_pretty(cfg).context("serialize config")?;
    fs::write(path, s).with_context(|| format!("write {}", path.display()))?;
    Ok(())
}

fn gen_token() -> String {
    let mut buf = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut buf);
    hex::encode(buf)
}

/// Resolve the ElevenLabs API key: env → ~/.bashrc export → config file.
pub fn resolve_elevenlabs_api_key(cfg: &Config) -> Option<String> {
    resolve_env_or_bashrc("ELEVENLABS_API_KEY")
        .or_else(|| cfg.elevenlabs.api_key.clone().filter(|k| !k.is_empty()))
}

/// Resolve the Groq API key: env → ~/.bashrc export → config file.
pub fn resolve_groq_api_key(cfg: &Config) -> Option<String> {
    resolve_env_or_bashrc("GROQ_API_KEY")
        .or_else(|| cfg.groq.api_key.clone().filter(|k| !k.is_empty()))
}

/// Backward-compatible alias for the original ElevenLabs-only resolver.
pub fn resolve_api_key(cfg: &Config) -> Option<String> {
    resolve_elevenlabs_api_key(cfg)
}

fn resolve_env_or_bashrc(name: &str) -> Option<String> {
    if let Ok(k) = std::env::var(name) {
        if !k.is_empty() {
            return Some(k);
        }
    }
    key_from_bashrc(name)
}

fn key_from_bashrc(name: &str) -> Option<String> {
    let path = home().join(".bashrc");
    let raw = fs::read_to_string(path).ok()?;
    for line in raw.lines() {
        let t = line.trim();
        if let Some(rest) = t.strip_prefix("export") {
            let rest = rest.trim();
            if let Some(kv) = rest.strip_prefix(name) {
                if let Some(kv) = kv.trim_start().strip_prefix('=') {
                    let v = kv.trim().trim_matches('"').trim_matches('\'').to_string();
                    if !v.is_empty() {
                        return Some(v);
                    }
                }
            }
        }
    }
    None
}

fn parse_bool(key: &str, value: &str) -> Result<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "true" | "1" | "yes" | "on" => Ok(true),
        "false" | "0" | "no" | "off" => Ok(false),
        _ => bail!("{key} must be a boolean"),
    }
}

fn parse_f32(key: &str, value: &str) -> Result<f32> {
    value
        .parse::<f32>()
        .with_context(|| format!("{key} must be a number"))
}

fn parse_f64(key: &str, value: &str) -> Result<f64> {
    value
        .parse::<f64>()
        .with_context(|| format!("{key} must be a number"))
}

fn parse_u32(key: &str, value: &str) -> Result<u32> {
    value
        .parse::<u32>()
        .with_context(|| format!("{key} must be an unsigned integer"))
}

fn parse_groq_sample_rate(key: &str, value: &str) -> Result<u32> {
    let sample_rate = parse_u32(key, value)?;
    if [8000, 16000, 22050, 24000, 32000, 44100, 48000].contains(&sample_rate) {
        Ok(sample_rate)
    } else {
        bail!("{key} must be one of 8000, 16000, 22050, 24000, 32000, 44100, or 48000")
    }
}

fn parse_u64(key: &str, value: &str) -> Result<u64> {
    value
        .parse::<u64>()
        .with_context(|| format!("{key} must be an unsigned integer"))
}

fn apply_voice_setting(settings: &mut SettingsPatch, field: &str, value: &str) -> Result<()> {
    match field {
        "stability" => settings.stability = Some(parse_f32(field, value)?),
        "similarity_boost" => settings.similarity_boost = Some(parse_f32(field, value)?),
        "style" => settings.style = Some(parse_f32(field, value)?),
        "speed" => settings.speed = Some(parse_f32(field, value)?),
        "use_speaker_boost" => settings.use_speaker_boost = Some(parse_bool(field, value)?),
        _ => bail!("unknown voice setting field {field}"),
    }
    Ok(())
}
