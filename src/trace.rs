//! Optional structured JSONL tracing sink for observing mimic efficiency.
//!
//! Enabled via `VOXD_TRACE_JSONL=<path>`; every tracing event (from any
//! target) is appended to that file as one flat JSON object per line, so
//! external tools (e.g. `tools/mimic_bench.py`) can reconstruct per-request
//! plan/admission/compose detail that the `/speak` HTTP response alone
//! doesn't expose.

use serde_json::{Map, Value};
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::sync::Mutex;
use tracing::field::{Field, Visit};
use tracing::Subscriber;
use tracing_subscriber::layer::Context;
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::Layer;

pub struct JsonlLayer {
    file: Mutex<File>,
}

impl JsonlLayer {
    pub fn new(path: impl AsRef<Path>) -> std::io::Result<Self> {
        let file = OpenOptions::new().create(true).append(true).open(path)?;
        Ok(Self {
            file: Mutex::new(file),
        })
    }

    /// Build from `VOXD_TRACE_JSONL`, if set. Logs and returns `None` on
    /// failure to open the path so a bad env var never blocks startup.
    pub fn from_env() -> Option<Self> {
        let path = std::env::var_os("VOXD_TRACE_JSONL")?;
        match Self::new(&path) {
            Ok(layer) => Some(layer),
            Err(e) => {
                eprintln!("voxd: VOXD_TRACE_JSONL={path:?}: {e}");
                None
            }
        }
    }
}

#[derive(Default)]
struct JsonVisitor(Map<String, Value>);

impl Visit for JsonVisitor {
    fn record_f64(&mut self, field: &Field, value: f64) {
        self.0.insert(field.name().to_string(), value.into());
    }
    fn record_i64(&mut self, field: &Field, value: i64) {
        self.0.insert(field.name().to_string(), value.into());
    }
    fn record_u64(&mut self, field: &Field, value: u64) {
        self.0.insert(field.name().to_string(), value.into());
    }
    fn record_bool(&mut self, field: &Field, value: bool) {
        self.0.insert(field.name().to_string(), value.into());
    }
    fn record_str(&mut self, field: &Field, value: &str) {
        self.0.insert(field.name().to_string(), value.into());
    }
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        self.0
            .insert(field.name().to_string(), format!("{value:?}").into());
    }
}

impl<S> Layer<S> for JsonlLayer
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_event(&self, event: &tracing::Event<'_>, _ctx: Context<'_, S>) {
        let mut visitor = JsonVisitor::default();
        event.record(&mut visitor);
        let meta = event.metadata();
        let mut obj = visitor.0;
        obj.insert("timestamp".into(), chrono::Utc::now().to_rfc3339().into());
        obj.insert("level".into(), meta.level().to_string().into());
        obj.insert("target".into(), meta.target().into());
        let line = match serde_json::to_string(&Value::Object(obj)) {
            Ok(l) => l,
            Err(_) => return,
        };
        if let Ok(mut f) = self.file.lock() {
            let _ = writeln!(f, "{line}");
            let _ = f.flush();
        }
    }
}
