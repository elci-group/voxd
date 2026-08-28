use voxd::state::{Db, MimicEfficiencySummary, MimicSynthesisRecord};

#[test]
fn log_mimic_synthesis_persists_and_summarizes() {
    let tmp = tempfile::tempdir().unwrap();
    let db = Db::open(&tmp.path().join("state.db")).unwrap();

    let voice = "voice-a".to_string();
    let model = "model-x".to_string();
    let project = "proj-1".to_string();

    // composed: 100 total, 70 cached, 30 billed -> 70% cache hit, 70% savings
    let r1 = MimicSynthesisRecord {
        plan_id: Some("plan-1"),
        project_id: Some(&project),
        voice_id: &voice,
        model_id: &model,
        total_chars: 100,
        cached_chars: 70,
        missing_chars: 30,
        provider_chars: 30,
        ram_admitted: true,
        storage_admitted: true,
        outcome: "composed",
        elapsed_ms: 12.5,
    };
    db.log_mimic_synthesis(&r1).unwrap();

    // ram_denied: 50 total, 10 cached, 40 missing, 0 billed -> 20% cache hit, 100% savings
    let r2 = MimicSynthesisRecord {
        plan_id: Some("plan-2"),
        project_id: None,
        voice_id: &voice,
        model_id: &model,
        total_chars: 50,
        cached_chars: 10,
        missing_chars: 40,
        provider_chars: 0,
        ram_admitted: false,
        storage_admitted: true,
        outcome: "ram_denied",
        elapsed_ms: 3.0,
    };
    db.log_mimic_synthesis(&r2).unwrap();

    // error before plan: zero chars -> must not panic or divide-by-zero
    let r3 = MimicSynthesisRecord {
        plan_id: None,
        project_id: None,
        voice_id: &voice,
        model_id: &model,
        total_chars: 0,
        cached_chars: 0,
        missing_chars: 0,
        provider_chars: 0,
        ram_admitted: false,
        storage_admitted: false,
        outcome: "error",
        elapsed_ms: 1.0,
    };
    db.log_mimic_synthesis(&r3).unwrap();

    let summary = db.mimic_efficiency_summary().unwrap();
    assert_eq!(summary.total, 3);
    assert_eq!(summary.composed, 1);
    assert_eq!(summary.ram_denied, 1);
    assert_eq!(summary.error, 1);

    // avg cache hit = (70/100 + 10/50 + 0) / 3 = (0.7 + 0.2 + 0) / 3 = 0.3 -> 30%
    let expected_cache_hit = ((70.0 / 100.0) + (10.0 / 50.0)) / 3.0 * 100.0;
    // avg savings = (70/100 + 50/50 + 0) / 3 = (0.7 + 1.0 + 0) / 3 = 0.5666... -> 56.67%
    let expected_savings = ((70.0 / 100.0) + 1.0) / 3.0 * 100.0;

    let diff = |a: f64, b: f64| (a - b).abs();
    assert!(
        diff(summary.avg_cache_hit_pct, expected_cache_hit) < 0.001,
        "avg_cache_hit_pct {} != {}",
        summary.avg_cache_hit_pct,
        expected_cache_hit
    );
    assert!(
        diff(summary.avg_savings_pct, expected_savings) < 0.001,
        "avg_savings_pct {} != {}",
        summary.avg_savings_pct,
        expected_savings
    );
}

#[test]
fn empty_db_summary_is_zero() {
    let tmp = tempfile::tempdir().unwrap();
    let db = Db::open(&tmp.path().join("state.db")).unwrap();
    let summary = db.mimic_efficiency_summary().unwrap();
    assert_eq!(summary, MimicEfficiencySummary::default());
}
