use voxd::listen::wake::{normalize, WakeMatcher};

#[test]
fn normalize_strips_punct_and_lowercases() {
    assert_eq!(
        normalize("Hey, Vox T. What TIME is it?"),
        "hey vox t what time is it"
    );
}

#[test]
fn exact_wake_returns_command() {
    let m = WakeMatcher::new("hey voxd");
    assert_eq!(
        m.check("Hey Voxd what time is it").as_deref(),
        Some("what time is it")
    );
}

#[test]
fn fuzzy_mishear_vox_t_returns_command() {
    let m = WakeMatcher::new("hey voxd");
    // Scribe rendered "Hey Voxd" as "Hey, Vox T."
    assert_eq!(
        m.check("Hey, Vox T. What time is it?").as_deref(),
        Some("What time is it?")
    );
}

#[test]
fn fuzzy_vox_dee_returns_command() {
    let m = WakeMatcher::new("hey voxd");
    assert_eq!(m.check("hey vox dee status").as_deref(), Some("status"));
}

#[test]
fn no_wake_returns_none() {
    let m = WakeMatcher::new("hey voxd");
    assert_eq!(m.check("what time is it"), None);
}

#[test]
fn wake_only_yields_empty_command() {
    let m = WakeMatcher::new("hey voxd");
    assert_eq!(m.check("Hey Voxd").as_deref(), Some(""));
}
