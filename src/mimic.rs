use crate::config::{expand_tilde, MimicCfg};
use crate::Settings;
use anyhow::{anyhow, bail, Context, Result};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use tokio::process::Command;

#[derive(Debug, Clone, Deserialize)]
pub struct Span {
    pub span_id: String,
    pub kind: String,
    pub text: String,
    pub link: Option<String>,
    pub sha256: Option<String>,
    pub bytes: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Plan {
    pub plan_id: String,
    pub expires_at: u64,
    pub manifest_dir: String,
    pub total_chars: usize,
    pub cached_chars: usize,
    pub missing_chars: usize,
    pub estimated_ram_bytes: u64,
    pub estimated_storage_bytes: u64,
    pub spans: Vec<Span>,
}

#[derive(Serialize)]
struct PlanRequest<'a> {
    text: &'a str,
    voice_id: &'a str,
    model_id: &'a str,
    settings_key: String,
}

#[derive(Debug, Deserialize)]
struct PvResource {
    admitted: bool,
}
#[derive(Debug, Deserialize)]
struct PvAdmission {
    ram: PvResource,
    storage: PvResource,
}

#[derive(Clone)]
pub struct MimicClient {
    http: Client,
    base: String,
    token: String,
    object_root: PathBuf,
    pv_bin: String,
}

impl MimicClient {
    pub fn new(http: Client, cfg: &MimicCfg, fallback_token: &str) -> Self {
        Self {
            http,
            base: cfg.url.trim_end_matches('/').into(),
            token: if cfg.auth_token.is_empty() {
                fallback_token.to_owned()
            } else {
                cfg.auth_token.clone()
            },
            object_root: expand_tilde(&cfg.object_root),
            pv_bin: cfg.pv_bin.clone(),
        }
    }

    #[tracing::instrument(skip_all, fields(voice = %voice, text_len = text.len()))]
    pub async fn plan(
        &self,
        text: &str,
        voice: &str,
        model: &str,
        settings: &Settings,
    ) -> Result<Plan> {
        let started = std::time::Instant::now();
        let response = self
            .http
            .post(format!("{}/v1/plans", self.base))
            .bearer_auth(&self.token)
            .json(&PlanRequest {
                text,
                voice_id: voice,
                model_id: model,
                settings_key: settings.cache_fragment(),
            })
            .send()
            .await
            .context("mimic plan request")?;
        if !response.status().is_success() {
            bail!("mimic plan HTTP {}", response.status());
        }
        let plan: Plan = response.json().await.context("parse mimic plan")?;
        self.validate(&plan)?;

        let missing_span_count = plan.spans.iter().filter(|s| s.kind == "missing").count();
        let cache_hit_pct = if plan.total_chars > 0 {
            plan.cached_chars as f64 * 100.0 / plan.total_chars as f64
        } else {
            0.0
        };
        tracing::info!(
            target: "voxd::mimic",
            event = "plan",
            plan_id = %plan.plan_id,
            total_chars = plan.total_chars,
            cached_chars = plan.cached_chars,
            missing_chars = plan.missing_chars,
            cache_hit_pct,
            span_count = plan.spans.len(),
            cached_span_count = plan.spans.len() - missing_span_count,
            missing_span_count,
            elapsed_ms = started.elapsed().as_secs_f64() * 1000.0,
            "mimic plan"
        );
        Ok(plan)
    }

    fn validate(&self, plan: &Plan) -> Result<()> {
        let root =
            std::fs::canonicalize(&self.object_root).context("canonicalize Mimic object root")?;
        let manifest =
            std::fs::canonicalize(&plan.manifest_dir).context("canonicalize Mimic manifest")?;
        for span in &plan.spans {
            if span.kind != "cached" {
                continue;
            }
            let link = span
                .link
                .as_deref()
                .ok_or_else(|| anyhow!("cached span has no link"))?;
            if Path::new(link).components().count() != 1 {
                bail!("non-local manifest link");
            }
            let target =
                std::fs::canonicalize(manifest.join(link)).context("resolve manifest link")?;
            if !target.starts_with(&root) {
                bail!("manifest link escaped object root");
            }
            let bytes = std::fs::read(target)?;
            if bytes.len() as u64 != span.bytes {
                bail!("manifest size mismatch");
            }
            let digest = format!("{:x}", Sha256::digest(&bytes));
            if span.sha256.as_deref() != Some(&digest) {
                bail!("manifest checksum mismatch");
            }
        }
        Ok(())
    }

    #[tracing::instrument(skip_all, fields(plan_id = %plan.plan_id))]
    pub async fn admit(&self, plan: &Plan) -> Result<(bool, bool)> {
        let started = std::time::Instant::now();
        let output = Command::new(&self.pv_bin)
            .args([
                "admit",
                "--ram-bytes",
                &plan.estimated_ram_bytes.to_string(),
                "--storage-bytes",
                &plan.estimated_storage_bytes.to_string(),
                "--path",
                self.object_root.to_str().unwrap_or("/"),
                "--format",
                "json",
            ])
            .output()
            .await
            .context("run pv admit")?;
        let report: PvAdmission =
            serde_json::from_slice(&output.stdout).context("parse pv admission")?;
        tracing::info!(
            target: "voxd::mimic",
            event = "admit",
            plan_id = %plan.plan_id,
            ram_admitted = report.ram.admitted,
            storage_admitted = report.storage.admitted,
            estimated_ram_bytes = plan.estimated_ram_bytes,
            estimated_storage_bytes = plan.estimated_storage_bytes,
            elapsed_ms = started.elapsed().as_secs_f64() * 1000.0,
            "mimic admit"
        );
        Ok((report.ram.admitted, report.storage.admitted))
    }

    #[tracing::instrument(skip_all, fields(plan_id = %plan, span_id = %span, bytes = pcm.len()))]
    pub async fn inject(&self, plan: &str, span: &str, pcm: Vec<u8>) -> Result<()> {
        let started = std::time::Instant::now();
        let bytes_len = pcm.len();
        let response = self
            .http
            .put(format!("{}/v1/plans/{plan}/spans/{span}", self.base))
            .bearer_auth(&self.token)
            .header("content-type", "audio/pcm")
            .body(pcm)
            .send()
            .await?;
        if !response.status().is_success() {
            bail!("mimic inject HTTP {}", response.status());
        }
        tracing::debug!(
            target: "voxd::mimic",
            event = "inject",
            plan_id = %plan,
            span_id = %span,
            bytes = bytes_len,
            elapsed_ms = started.elapsed().as_secs_f64() * 1000.0,
            "mimic inject span"
        );
        Ok(())
    }

    #[tracing::instrument(skip_all, fields(plan_id = %plan, persist))]
    pub async fn compose(&self, plan: &str, persist: bool) -> Result<Vec<u8>> {
        let started = std::time::Instant::now();
        let response = self
            .http
            .post(format!(
                "{}/v1/plans/{plan}/compose?persist={persist}",
                self.base
            ))
            .bearer_auth(&self.token)
            .send()
            .await?;
        if !response.status().is_success() {
            bail!("mimic compose HTTP {}", response.status());
        }
        let bytes = response.bytes().await?.to_vec();
        tracing::info!(
            target: "voxd::mimic",
            event = "compose",
            plan_id = %plan,
            persist,
            audio_bytes = bytes.len(),
            elapsed_ms = started.elapsed().as_secs_f64() * 1000.0,
            "mimic compose"
        );
        Ok(bytes)
    }
}
