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
    pub fn new(http: Client, cfg: &MimicCfg) -> Self {
        Self {
            http,
            base: cfg.url.trim_end_matches('/').into(),
            token: cfg.auth_token.clone(),
            object_root: expand_tilde(&cfg.object_root),
            pv_bin: cfg.pv_bin.clone(),
        }
    }

    pub async fn plan(
        &self,
        text: &str,
        voice: &str,
        model: &str,
        settings: &Settings,
    ) -> Result<Plan> {
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

    pub async fn admit(&self, plan: &Plan) -> Result<(bool, bool)> {
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
        Ok((report.ram.admitted, report.storage.admitted))
    }

    pub async fn inject(&self, plan: &str, span: &str, pcm: Vec<u8>) -> Result<()> {
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
        Ok(())
    }

    pub async fn compose(&self, plan: &str, persist: bool) -> Result<Vec<u8>> {
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
        Ok(response.bytes().await?.to_vec())
    }
}
