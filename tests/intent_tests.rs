use voxd::listen::responder::{IntentRouter, Responder};

#[test]
fn time_intent_has_clock_and_is_low_latency() {
    let r = IntentRouter.respond("what time is it");
    assert!(r.low_latency);
    assert!(!r.stop);
    let re = regex_lite(&r.text);
    assert!(re, "expected a clock in: {}", r.text);
}

#[test]
fn stop_intent_sets_stop() {
    let r = IntentRouter.respond("hey please stop listening");
    assert!(r.stop);
}

#[test]
fn empty_command_prompts() {
    let r = IntentRouter.respond("");
    assert_eq!(r.text, "Yes?");
}

#[test]
fn fallback_echoes_command() {
    let r = IntentRouter.respond("tell me a joke");
    assert!(r.text.starts_with("You said:"));
    assert!(r.text.contains("tell me a joke"));
}

#[test]
fn specs_intent_mentions_system() {
    let r = IntentRouter.respond("give me the system specs");
    assert!(r.text.contains("System:") || r.text.contains("unavailable"));
}

// Tiny check: does text contain something like HH:MM?
fn regex_lite(s: &str) -> bool {
    let bytes = s.as_bytes();
    for w in bytes.windows(5) {
        if w[0].is_ascii_digit()
            && w[1].is_ascii_digit()
            && w[2] == b':'
            && w[3].is_ascii_digit()
            && w[4].is_ascii_digit()
        {
            return true;
        }
    }
    false
}
