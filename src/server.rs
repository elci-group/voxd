use std::collections::HashSet;
use std::sync::Arc;
use std::time::Instant;

use anyhow::{anyhow, Context, Result};
use axum::{
    body::Bytes,
    extract::{Json, State},
    http::{header::AUTHORIZATION, HeaderMap, Request, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
    Router,
};
use serde::{Deserialize, Serialize};

use crate::alloc::allocate_voice;
use crate::cache::AudioCache;
use crate::config::{resolve_elevenlabs_api_key, resolve_groq_api_key, Config, SpeechProvider};
use crate::elevenlabs::ElevenClient;
use crate::groq::GroqClient;
use crate::state::Db;
use crate::{play, project, ProjectRow, Settings, SettingsPatch};

pub struct AppState {
    pub(crate) cfg: Config,
    elevenlabs_key: Option<String>,
    groq_key: Option<String>,
    http: reqwest::Client,
    db: Db,
    cache: AudioCache,
    started: Instant,
    listen: tokio::sync::Mutex<Option<crate::listen::ListenHandle>>,
}

impl AppState {
    pub fn new(cfg: Config) -> Result<Self> {
        if cfg.providers.tts == SpeechProvider::Groq {
            if cfg.groq.output_format != "wav" {
                return Err(anyhow!("Groq Orpheus requires groq.output_format = 'wav'"));
            }
            if !crate::groq::supports_voice(&cfg.groq.tts_model, &cfg.groq.voice) {
                return Err(anyhow!(
                    "Groq voice '{}' is not valid for model '{}'",
                    cfg.groq.voice,
                    cfg.groq.tts_model
                ));
            }
            if ![8000, 16000, 22050, 24000, 32000, 44100, 48000].contains(&cfg.groq.sample_rate) {
                return Err(anyhow!(
                    "unsupported Groq sample rate {}; expected 8000, 16000, 22050, 24000, 32000, 44100, or 48000",
                    cfg.groq.sample_rate
                ));
            }
        }
        let elevenlabs_key = resolve_elevenlabs_api_key(&cfg);
        let groq_key = resolve_groq_api_key(&cfg);
        let http = reqwest::Client::builder()
            .user_agent("voxd/0.1")
            .build()
            .context("http client")?;
        let db = Db::open(&cfg.state_db())?;
        let cache = AudioCache::new(cfg.cache_dir(), cfg.cache.enabled, cfg.cache.max_mb)?;
        Ok(Self {
            cfg,
            elevenlabs_key,
            groq_key,
            http,
            db,
            cache,
            started: Instant::now(),
            listen: tokio::sync::Mutex::new(None),
        })
    }

    pub(crate) fn eleven(&self) -> Result<ElevenClient> {
        let key = self.elevenlabs_key.clone().ok_or_else(|| {
            anyhow!("no ElevenLabs API key (set ELEVENLABS_API_KEY or configure one)")
        })?;
        Ok(ElevenClient::new(
            self.http.clone(),
            key,
            self.cfg.elevenlabs.model_id.clone(),
            self.cfg.elevenlabs.output_format.clone(),
        ))
    }

    pub(crate) fn groq(&self) -> Result<GroqClient> {
        let key = self
            .groq_key
            .clone()
            .ok_or_else(|| anyhow!("no Groq API key (set GROQ_API_KEY or configure one)"))?;
        Ok(GroqClient::new(
            self.http.clone(),
            key,
            self.cfg.groq.tts_model.clone(),
            self.cfg.groq.output_format.clone(),
            self.cfg.groq.sample_rate,
            self.cfg.groq.stt_model.clone(),
        ))
    }

    pub(crate) fn tts_model(&self) -> &str {
        match self.cfg.providers.tts {
            SpeechProvider::Elevenlabs => &self.cfg.elevenlabs.model_id,
            SpeechProvider::Groq => &self.cfg.groq.tts_model,
        }
    }

    pub(crate) fn tts_format(&self) -> &str {
        match self.cfg.providers.tts {
            SpeechProvider::Elevenlabs => &self.cfg.elevenlabs.output_format,
            SpeechProvider::Groq => &self.cfg.groq.output_format,
        }
    }

    pub(crate) fn resolve_tts_voice(&self, requested: &str) -> String {
        match self.cfg.providers.tts {
            SpeechProvider::Elevenlabs => requested.to_string(),
            SpeechProvider::Groq => {
                if crate::groq::supports_voice(&self.cfg.groq.tts_model, requested) {
                    requested.to_string()
                } else {
                    self.cfg.groq.voice.clone()
                }
            }
        }
    }

    pub(crate) async fn synthesize(
        &self,
        text: &str,
        voice: &str,
        settings: &Settings,
    ) -> Result<Vec<u8>> {
        match self.cfg.providers.tts {
            SpeechProvider::Elevenlabs => self.eleven()?.speak(text, voice, settings).await,
            SpeechProvider::Groq => self.groq()?.speak(text, voice, settings).await,
        }
    }

    pub(crate) async fn transcribe(&self, audio: Vec<u8>, mime: &str) -> Result<String> {
        match self.cfg.providers.stt {
            SpeechProvider::Elevenlabs => {
                self.eleven()?
                    .transcribe(audio, mime, &self.cfg.listen.stt_model)
                    .await
            }
            SpeechProvider::Groq => self.groq()?.transcribe(audio, mime).await,
        }
    }
}

pub async fn run(cfg: Config) -> Result<()> {
    let bind = cfg.server.bind.clone();
    let state = Arc::new(AppState::new(cfg)?);
    crate::recap::spawn(state.clone());

    let protected = Router::new()
        .route("/speak", post(handle_speak))
        .route("/voices", get(handle_voices))
        .route("/projects", get(handle_projects))
        .route("/projects/assign", post(handle_assign))
        .route("/projects/unassign", post(handle_unassign))
        .route("/listen/start", post(listen_start))
        .route("/listen/stop", post(listen_stop))
        .route("/listen/status", get(listen_status))
        .route("/listen/transcribe", post(listen_transcribe))
        .route("/status", get(handle_status));

    let app = Router::new()
        .route("/health", get(handle_health))
        .merge(protected)
        .layer(middleware::from_fn_with_state(state.clone(), require_auth))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(&bind)
        .await
        .with_context(|| format!("bind {bind}"))?;
    tracing::info!(%bind, "voxd listening");
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .context("serve")?;
    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let term = async {
        use tokio::signal::unix::{signal, SignalKind};
        if let Ok(mut s) = signal(SignalKind::terminate()) {
            s.recv().await;
        }
    };
    #[cfg(not(unix))]
    let term = std::future::pending::<()>();
    tokio::select! { _ = ctrl_c => {}, _ = term => {} }
}

async fn require_auth(
    State(state): State<Arc<AppState>>,
    req: Request<axum::body::Body>,
    next: Next,
) -> Result<Response, StatusCode> {
    if req.uri().path() == "/health" {
        return Ok(next.run(req).await);
    }
    let expected = format!("Bearer {}", state.cfg.server.auth_token);
    let ok = req
        .headers()
        .get(AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .map(|v| v == expected)
        .unwrap_or(false);
    if ok {
        Ok(next.run(req).await)
    } else {
        Err(StatusCode::UNAUTHORIZED)
    }
}

// ---- handlers -------------------------------------------------------------

async fn handle_health() -> impl IntoResponse {
    Json(serde_json::json!({ "ok": true }))
}

async fn handle_status(State(s): State<Arc<AppState>>) -> impl IntoResponse {
    let uptime = s.started.elapsed().as_secs();
    let utterances = s.db.utterance_count().unwrap_or(0);
    let projects = s.db.list_projects().map(|v| v.len()).unwrap_or(0);
    let cache_bytes = s.cache.total_bytes();
    Json(serde_json::json!({
        "ok": true,
        "uptime_secs": uptime,
        "projects": projects,
        "utterances": utterances,
        "cache_bytes": cache_bytes,
        "cache_dir": s.cfg.cache_dir().display().to_string(),
        "key_present": match s.cfg.providers.tts {
            SpeechProvider::Elevenlabs => s.elevenlabs_key.is_some(),
            SpeechProvider::Groq => s.groq_key.is_some(),
        },
        "elevenlabs_key_present": s.elevenlabs_key.is_some(),
        "groq_key_present": s.groq_key.is_some(),
        "tts_provider": s.cfg.providers.tts.to_string(),
        "stt_provider": s.cfg.providers.stt.to_string(),
        "system_voice": s.resolve_tts_voice(&s.cfg.system_voice.voice_id),
        "model": s.tts_model(),
        "tts_model": s.tts_model(),
        "stt_model": match s.cfg.providers.stt {
            SpeechProvider::Elevenlabs => &s.cfg.listen.stt_model,
            SpeechProvider::Groq => &s.cfg.groq.stt_model,
        },
    }))
}

#[derive(Deserialize)]
struct SpeakReq {
    text: String,
    #[serde(default)]
    project_path: Option<String>,
    #[serde(default)]
    project_id: Option<String>,
    #[serde(default)]
    system: bool,
    #[serde(default)]
    voice_id: Option<String>,
    #[serde(default)]
    label: Option<String>,
    #[serde(default)]
    play: bool,
    #[serde(default)]
    no_cache: bool,
    #[serde(default)]
    settings: SettingsPatch,
}

#[derive(Serialize)]
struct SpeakResp {
    voice_id: String,
    label: String,
    project_id: Option<String>,
    cached: bool,
    chars: usize,
    audio_path: String,
    visual_only: bool,
    provider_chars: usize,
}

/// Speak harness-detected recap text without going through HTTP. Used by the
/// background recap watcher (see `crate::recap`); best-effort, so callers just
/// log a failure rather than propagate it.
pub(crate) async fn speak_recap(
    s: &AppState,
    text: String,
    project_path: Option<String>,
) -> Result<()> {
    let req = SpeakReq {
        text,
        system: project_path.is_none(),
        project_path,
        project_id: None,
        voice_id: None,
        label: None,
        play: true,
        no_cache: false,
        settings: SettingsPatch::default(),
    };
    speak_core(s, req).await?;
    Ok(())
}

async fn handle_speak(State(s): State<Arc<AppState>>, Json(req): Json<SpeakReq>) -> Response {
    match speak_core(&s, req).await {
        Ok(resp) => Json(resp).into_response(),
        Err(e) => {
            tracing::warn!(error = %e, "speak failed");
            (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
                .into_response()
        }
    }
}

async fn speak_core(s: &AppState, req: SpeakReq) -> Result<SpeakResp> {
    let text = req.text.trim().to_string();
    if text.is_empty() {
        return Err(anyhow!("empty text"));
    }
    let chars = text.chars().count();

    // Resolve voice / label / settings / project binding.
    let (voice_id, label, settings, project_id): (String, String, Settings, Option<String>) =
        if req.system {
            let v = req
                .voice_id
                .unwrap_or_else(|| s.cfg.system_voice.voice_id.clone());
            let l = req
                .label
                .unwrap_or_else(|| s.cfg.system_voice.label.clone());
            let set = s.cfg.defaults.apply(&req.settings);
            (v, l, set, None)
        } else {
            let row = match &req.project_id {
                Some(id) => {
                    s.db.get_project_by_id(id)?
                        .ok_or_else(|| anyhow!("unknown project id {id}"))?
                }
                None => get_or_allocate(s, req.project_path.as_deref()).await?,
            };
            let v = req.voice_id.unwrap_or_else(|| row.voice_id.clone());
            let l = req.label.unwrap_or_else(|| row.label.clone());
            let set = row.settings.apply(&req.settings);
            (v, l, set, Some(row.id.clone()))
        };
    let voice_id = s.resolve_tts_voice(&voice_id);

    // Cache lookup / synthesis.
    let cache_model = format!("{}:{}", s.cfg.providers.tts, s.tts_model());
    let key = s
        .cache
        .key(&text, &voice_id, &cache_model, s.tts_format(), &settings);
    let audio_path = s.cache.path_for(&key);

    let cached = if !req.no_cache {
        s.cache.get(&key)
    } else {
        None
    };
    let (cached, audio_path, visual_only, provider_chars) = match cached {
        Some(bytes) => {
            std::fs::write(&audio_path, &bytes).ok();
            (true, audio_path, false, 0)
        }
        None => {
            if s.cfg.mimic.enabled && s.cfg.providers.tts == SpeechProvider::Elevenlabs {
                match synthesize_with_mimic(s, &text, &voice_id, &settings).await {
                    Ok(Some((bytes, provider_chars))) => {
                        let path = s.cache.put_wav(&key, &bytes)?;
                        (false, path, false, provider_chars)
                    }
                    Ok(None) => {
                        crate::notify::intended_message(&text);
                        (false, std::path::PathBuf::new(), true, 0)
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "mimic preflight failed; blocking TTS");
                        crate::notify::intended_message(&text);
                        (false, std::path::PathBuf::new(), true, 0)
                    }
                }
            } else {
                let bytes = s.synthesize(&text, &voice_id, &settings).await?;
                let path = if s.cfg.providers.tts == SpeechProvider::Groq {
                    s.cache.put_wav(&key, &bytes)?
                } else if s.cfg.cache.enabled {
                    s.cache.put(&key, &bytes)?
                } else {
                    std::fs::write(&audio_path, &bytes)?;
                    audio_path
                };
                (false, path, false, chars)
            }
        }
    };

    s.db.log_utterance(project_id.as_deref(), &voice_id, chars, cached)?;

    if req.play && !visual_only {
        play::spawn_play(&audio_path);
    }

    Ok(SpeakResp {
        voice_id,
        label,
        project_id,
        cached,
        chars,
        audio_path: audio_path.display().to_string(),
        visual_only,
        provider_chars,
    })
}

/// Execute the mandatory Mimic manifest and pv admission flow. `None` means
/// RAM was denied and the caller must use a visual-only response.
pub(crate) async fn synthesize_with_mimic(
    s: &AppState,
    text: &str,
    voice_id: &str,
    settings: &Settings,
) -> Result<Option<(Vec<u8>, usize)>> {
    let mimic =
        crate::mimic::MimicClient::new(s.http.clone(), &s.cfg.mimic, &s.cfg.server.auth_token);
    let plan = mimic
        .plan(text, voice_id, &s.cfg.elevenlabs.model_id, settings)
        .await?;
    let (ram_ok, storage_ok) = mimic.admit(&plan).await?;
    if !ram_ok {
        return Ok(None);
    }
    let eleven = s.eleven()?;
    for span in plan.spans.iter().filter(|span| span.kind == "missing") {
        let pcm = eleven.speak_pcm(&span.text, voice_id, settings).await?;
        mimic.inject(&plan.plan_id, &span.span_id, pcm).await?;
    }
    let audio = mimic.compose(&plan.plan_id, storage_ok).await?;
    Ok(Some((audio, plan.missing_chars)))
}

/// Resolve a project by path (or cwd), allocating and persisting a voice on
/// first sight.
async fn get_or_allocate(s: &AppState, path: Option<&str>) -> Result<ProjectRow> {
    let probe = match path {
        Some(p) => std::path::PathBuf::from(p),
        None => std::env::current_dir().context("cwd")?,
    };
    let pref = project::resolve(&probe)?;

    if let Some(row) = s.db.get_project_by_root(&pref.root_path)? {
        return Ok(row);
    }

    // Build the active provider's pool. Groq publishes a fixed Orpheus voice
    // list; ElevenLabs may use configured, cached, live, or built-in voices.
    let sys = s.resolve_tts_voice(&s.cfg.system_voice.voice_id);
    let mut pool = s.cfg.pool.voices.clone();
    if s.cfg.providers.tts == SpeechProvider::Groq {
        pool.retain(|voice| crate::groq::supports_voice(&s.cfg.groq.tts_model, voice));
        if pool.is_empty() {
            pool = crate::groq::voices(&s.cfg.groq.tts_model)
                .into_iter()
                .map(|voice| voice.voice_id)
                .collect();
        }
    } else {
        if pool.is_empty() {
            pool = s.db.cached_voice_ids()?;
        }
        if pool.is_empty() {
            match s.eleven() {
                Ok(client) => match client.list_voices().await {
                    Ok(voices) => {
                        s.db.upsert_voices(&voices)?;
                        pool = voices.into_iter().map(|v| v.voice_id).collect();
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "list_voices failed; using built-in pool");
                        pool = crate::voices::builtin_ids_excluding(&sys);
                    }
                },
                Err(e) => {
                    tracing::warn!(error = %e, "no api key for list_voices; using built-in pool");
                    pool = crate::voices::builtin_ids_excluding(&sys);
                }
            }
        }
    }
    // Never allocate the system voice to a project.
    let mut used: HashSet<String> =
        s.db.list_projects()?
            .into_iter()
            .map(|r| r.voice_id)
            .collect();
    used.insert(sys.clone());
    pool.retain(|v| v != &sys);

    let voice = allocate_voice(&pref.id, &pool, &used)
        .ok_or_else(|| anyhow!("no voices available to allocate (pool empty)"))?;

    let ts = now_rfc3339();
    let row = ProjectRow {
        id: pref.id,
        name: pref.name,
        root_path: pref.root_path,
        voice_id: voice,
        label: "auto".into(),
        settings: s.cfg.defaults,
        created_at: ts.clone(),
        updated_at: ts,
    };
    s.db.insert_project(&row)?;
    Ok(row)
}

async fn handle_voices(State(s): State<Arc<AppState>>) -> Response {
    if s.cfg.providers.tts == SpeechProvider::Groq {
        return Json(crate::groq::voices(&s.cfg.groq.tts_model)).into_response();
    }
    if let Ok(client) = s.eleven() {
        match client.list_voices().await {
            Ok(v) => {
                let _ = s.db.upsert_voices(&v);
                return Json(v).into_response();
            }
            Err(e) => tracing::warn!(error = %e, "live voices unavailable; serving built-in pool"),
        }
    }
    // Graceful fallback for restricted keys (no voices_read) or missing key.
    Json(crate::voices::builtin_voices()).into_response()
}

async fn handle_projects(State(s): State<Arc<AppState>>) -> Response {
    match s.db.list_projects() {
        Ok(v) => Json(v).into_response(),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    }
}

#[derive(Deserialize)]
struct AssignReq {
    project_id: String,
    #[serde(default)]
    project_path: Option<String>,
    voice_id: String,
    #[serde(default)]
    label: Option<String>,
    #[serde(default)]
    settings: SettingsPatch,
}

async fn handle_assign(State(s): State<Arc<AppState>>, Json(req): Json<AssignReq>) -> Response {
    if matches!(s.db.get_project_by_id(&req.project_id), Ok(None)) {
        if let Some(path) = req.project_path.as_deref() {
            match project::resolve(std::path::Path::new(path)) {
                Ok(pref) if pref.id == req.project_id => {
                    let ts = now_rfc3339();
                    let row = ProjectRow {
                        id: pref.id,
                        name: pref.name,
                        root_path: pref.root_path,
                        voice_id: req.voice_id.clone(),
                        label: req.label.clone().unwrap_or_else(|| "assigned".into()),
                        settings: s.cfg.defaults,
                        created_at: ts.clone(),
                        updated_at: ts,
                    };
                    if let Err(e) = s.db.insert_project(&row) {
                        return err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string());
                    }
                }
                Ok(_) => return err(StatusCode::BAD_REQUEST, "project id/path mismatch"),
                Err(e) => return err(StatusCode::BAD_REQUEST, &e.to_string()),
            }
        }
    }
    let new_settings = match s.db.get_project_by_id(&req.project_id) {
        Ok(Some(row)) => Some(row.settings.apply(&req.settings)),
        Ok(None) => return err(StatusCode::NOT_FOUND, "unknown project id"),
        Err(e) => return err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    };
    match s.db.assign(
        &req.project_id,
        &req.voice_id,
        req.label.as_deref(),
        new_settings,
    ) {
        Ok(true) => match s.db.get_project_by_id(&req.project_id) {
            Ok(Some(row)) => Json(row).into_response(),
            _ => err(
                StatusCode::INTERNAL_SERVER_ERROR,
                "row vanished after assign",
            ),
        },
        Ok(false) => err(StatusCode::NOT_FOUND, "unknown project id"),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    }
}

#[derive(Deserialize)]
struct UnassignReq {
    project_id: String,
}

async fn handle_unassign(State(s): State<Arc<AppState>>, Json(req): Json<UnassignReq>) -> Response {
    match s.db.delete_project(&req.project_id) {
        Ok(true) => {
            Json(serde_json::json!({ "deleted": true, "id": req.project_id })).into_response()
        }
        Ok(false) => err(StatusCode::NOT_FOUND, "unknown project id"),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    }
}

async fn listen_start(State(s): State<Arc<AppState>>) -> Response {
    let mut g = s.listen.lock().await;
    if g.is_some() {
        return err(StatusCode::CONFLICT, "listener already running");
    }
    match crate::listen::start(s.clone()).await {
        Ok(h) => {
            *g = Some(h);
            Json(serde_json::json!({ "started": true })).into_response()
        }
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    }
}

async fn listen_stop(State(s): State<Arc<AppState>>) -> Response {
    let h = {
        let mut g = s.listen.lock().await;
        g.take()
    };
    match h {
        Some(h) => {
            h.stop().await;
            Json(serde_json::json!({ "stopped": true })).into_response()
        }
        None => {
            Json(serde_json::json!({ "stopped": false, "reason": "not running" })).into_response()
        }
    }
}

async fn listen_status(State(s): State<Arc<AppState>>) -> Response {
    let g = s.listen.lock().await;
    Json(serde_json::json!({ "listening": g.is_some() })).into_response()
}

async fn listen_transcribe(
    State(s): State<Arc<AppState>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if body.is_empty() {
        return err(StatusCode::BAD_REQUEST, "empty audio body");
    }
    let mime = headers
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("audio/wav")
        .to_string();
    match s.transcribe(body.to_vec(), &mime).await {
        Ok(text) => Json(serde_json::json!({ "text": text })).into_response(),
        Err(e) => err(StatusCode::BAD_GATEWAY, &e.to_string()),
    }
}

fn err(status: StatusCode, msg: &str) -> Response {
    (status, Json(serde_json::json!({ "error": msg }))).into_response()
}

fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339()
}
