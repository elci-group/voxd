use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use rand::RngCore;
use serde::{Deserialize, Serialize};

use crate::Settings;

const DEFAULT_BIND: &str = "127.0.0.1:17843";
const DEFAULT_SYSTEM_VOICE: &str = "21m00Tcm4TlvDq8ikWAM"; // Rachel (premade)

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Config {
    #[serde(default)]
    pub server: ServerCfg,
    #[serde(default)]
    pub elevenlabs: ElevenCfg,
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
}

impl Default for MimicCfg {
    fn default() -> Self {
        Self {
            enabled: false,
            url: d_mimic_url(),
            auth_token: String::new(),
            pv_bin: d_pv_bin(),
            object_root: d_mimic_objects(),
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
            _ => bail!("unknown config key {key}"),
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
    // Backfill newly-added sections (e.g. [listen]) so the on-disk file reflects
    // current defaults and remains user-tunable.
    if !raw.contains("[listen]") && !wrote {
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
pub fn resolve_api_key(cfg: &Config) -> Option<String> {
    if let Ok(k) = std::env::var("ELEVENLABS_API_KEY") {
        if !k.is_empty() {
            return Some(k);
        }
    }
    if let Some(k) = key_from_bashrc() {
        return Some(k);
    }
    cfg.elevenlabs.api_key.clone().filter(|k| !k.is_empty())
}

fn key_from_bashrc() -> Option<String> {
    let path = home().join(".bashrc");
    let raw = fs::read_to_string(path).ok()?;
    for line in raw.lines() {
        let t = line.trim();
        if let Some(rest) = t.strip_prefix("export") {
            let rest = rest.trim();
            if let Some(kv) = rest.strip_prefix("ELEVENLABS_API_KEY") {
                let kv = kv.trim_start().strip_prefix('=')?.trim();
                let v = kv.trim_matches('"').trim_matches('\'').to_string();
                if !v.is_empty() {
                    return Some(v);
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

fn parse_u64(key: &str, value: &str) -> Result<u64> {
    value
        .parse::<u64>()
        .with_context(|| format!("{key} must be an unsigned integer"))
}
