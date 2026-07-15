use std::collections::HashSet;

use voxd::alloc::allocate_voice;
use voxd::state::Db;
use voxd::{ProjectRow, Settings};

#[test]
fn empty_pool_returns_none() {
    let used = HashSet::new();
    assert_eq!(allocate_voice("pid", &[], &used), None);
}

#[test]
fn allocation_is_deterministic() {
    let pool: Vec<String> = ["vA", "vB", "vC", "vD"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    let used = HashSet::new();
    let a = allocate_voice("project-x", &pool, &used);
    let b = allocate_voice("project-x", &pool, &used);
    assert_eq!(a, b);
    assert!(a.is_some());
}

#[test]
fn skips_used_voices() {
    let pool: Vec<String> = ["vA", "vB", "vC", "vD"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    let mut used = HashSet::new();
    used.insert("vA".to_string());
    // Ban vA for every project id we probe: result must never be vA.
    for id in ["p1", "p2", "p3", "p4", "p5", "p6", "p7", "p8"] {
        let v = allocate_voice(id, &pool, &used).unwrap();
        assert_ne!(v, "vA", "id {id} mapped to a used voice");
    }
}

#[test]
fn allocation_skips_persisted_binding_across_restart() {
    let tmp = tempfile::tempdir().unwrap();
    let db_path = tmp.path().join("state.db");

    // First run: bind project A to voice vA.
    {
        let db = Db::open(&db_path).unwrap();
        let ts = "2026-01-01T00:00:00Z".to_string();
        db.insert_project(&ProjectRow {
            id: "aaa".into(),
            name: "A".into(),
            root_path: "/repos/A".into(),
            voice_id: "vA".into(),
            label: "auto".into(),
            settings: Settings::default(),
            created_at: ts.clone(),
            updated_at: ts,
        })
        .unwrap();
    }

    // "Restart": reopen the same db, allocate for a new project B.
    let db = Db::open(&db_path).unwrap();
    let pool: Vec<String> = ["vA", "vB", "vC"].iter().map(|s| s.to_string()).collect();
    let used: HashSet<String> = db
        .list_projects()
        .unwrap()
        .into_iter()
        .map(|r| r.voice_id)
        .collect();
    assert!(used.contains("vA"));
    let v = allocate_voice("bbb", &pool, &used).unwrap();
    assert_ne!(v, "vA");
}
