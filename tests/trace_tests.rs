use std::io::Read;

use tracing_subscriber::layer::SubscriberExt;
use voxd::trace::JsonlLayer;

fn read_lines(path: &std::path::Path) -> Vec<String> {
    let mut contents = String::new();
    std::fs::File::open(path)
        .unwrap()
        .read_to_string(&mut contents)
        .unwrap();
    contents.lines().map(|l| l.to_string()).collect()
}

#[test]
fn emits_well_formed_jsonl_with_expected_fields() {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    let layer = JsonlLayer::new(tmp.path()).unwrap();
    let subscriber = tracing_subscriber::registry().with(layer);

    tracing::subscriber::with_default(subscriber, || {
        tracing::info!(
            target: "voxd::mimic",
            event = "plan",
            plan_id = "abc123",
            total_chars = 42usize,
            cached_chars = 30usize,
            missing_chars = 12usize,
            cache_hit_pct = 71.4_f64,
            ram_admitted = true,
            "mimic plan"
        );
    });

    let lines = read_lines(tmp.path());
    assert_eq!(lines.len(), 1);
    let v: serde_json::Value = serde_json::from_str(&lines[0]).expect("valid json line");

    assert_eq!(v["target"], "voxd::mimic");
    assert_eq!(v["level"], "INFO");
    assert_eq!(v["event"], "plan");
    assert_eq!(v["plan_id"], "abc123");
    assert_eq!(v["total_chars"], 42);
    assert_eq!(v["cached_chars"], 30);
    assert_eq!(v["missing_chars"], 12);
    assert_eq!(v["ram_admitted"], true);
    assert!(v["timestamp"].is_string());
    assert!(v["message"].is_string());
}

#[test]
fn multiple_events_produce_multiple_lines() {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    let layer = JsonlLayer::new(tmp.path()).unwrap();
    let subscriber = tracing_subscriber::registry().with(layer);

    tracing::subscriber::with_default(subscriber, || {
        tracing::info!(target: "voxd::speak", n = 1u64, "one");
        tracing::info!(target: "voxd::speak", n = 2u64, "two");
    });

    let lines = read_lines(tmp.path());
    assert_eq!(lines.len(), 2);
    let first: serde_json::Value = serde_json::from_str(&lines[0]).unwrap();
    let second: serde_json::Value = serde_json::from_str(&lines[1]).unwrap();
    assert_eq!(first["n"], 1);
    assert_eq!(second["n"], 2);
}

#[test]
fn missing_directory_does_not_panic() {
    assert!(JsonlLayer::new("/nonexistent/dir/trace.jsonl").is_err());
}
