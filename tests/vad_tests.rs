use voxd::listen::vad::{rms, Vad, FRAME_SAMPLES};

fn silent() -> Vec<i16> {
    vec![0; FRAME_SAMPLES]
}
fn loud() -> Vec<i16> {
    vec![10_000; FRAME_SAMPLES]
}

#[test]
fn rms_silent_vs_loud() {
    assert_eq!(rms(&silent()), 0.0);
    assert!(rms(&loud()) > 0.2);
}

#[test]
fn silence_only_never_emits() {
    let mut v = Vad::new(0.02, 100, 5, 3.0);
    for _ in 0..100 {
        assert!(v.feed(&silent()).is_none());
    }
}

#[test]
fn speech_then_silence_emits_utterance() {
    // silence_ms = 100 -> 5 frames to close
    let mut v = Vad::new(0.02, 100, 5, 3.0);
    for _ in 0..3 {
        assert!(v.feed(&loud()).is_none());
    }
    let mut emitted = None;
    for _ in 0..10 {
        if let Some(utt) = v.feed(&silent()) {
            emitted = Some(utt);
            break;
        }
    }
    let utt = emitted.expect("expected an utterance after trailing silence");
    assert!(utt.len() >= 3 * FRAME_SAMPLES, "utt len {}", utt.len());
}

#[test]
fn sustained_noise_adapts_floor_and_stops_triggering() {
    // Steady moderate noise (rms ≈ 0.09) is above the raw 0.02 threshold, so
    // the first burst rides out to the 1 s cap and emits exactly once.
    let noise = vec![3_000i16; FRAME_SAMPLES];
    assert!(rms(&noise) > 0.02 && rms(&noise) < 0.15);
    let mut v = Vad::new(0.02, 100, 1, 3.0);
    let mut emitted = false;
    for _ in 0..60 {
        if v.feed(&noise).is_some() {
            emitted = true;
            break;
        }
    }
    assert!(emitted, "first noisy burst should emit at the cap");
    // The floor snapped up: the same noise never triggers again.
    for _ in 0..300 {
        assert!(v.feed(&noise).is_none());
    }
    // A genuinely loud burst still gets through.
    let mut emitted = false;
    for _ in 0..60 {
        if v.feed(&loud()).is_some() {
            emitted = true;
            break;
        }
    }
    assert!(emitted, "loud speech should still trigger above the floor");
}

#[test]
fn floor_decays_back_in_silence() {
    let noise = vec![3_000i16; FRAME_SAMPLES];
    let mut v = Vad::new(0.02, 100, 1, 3.0);
    for _ in 0..60 {
        if v.feed(&noise).is_some() {
            break;
        }
    }
    // Long silence lets the floor decay; soft speech-level audio triggers
    // again even though it was below the noise-snapped floor * margin.
    for _ in 0..1500 {
        assert!(v.feed(&silent()).is_none());
    }
    let soft = vec![1_000i16; FRAME_SAMPLES]; // rms ≈ 0.03, above raw 0.02
    for _ in 0..3 {
        assert!(v.feed(&soft).is_none(), "still collecting");
    }
    let mut triggered = false;
    for _ in 0..10 {
        if v.feed(&silent()).is_some() {
            triggered = true;
            break;
        }
    }
    assert!(
        triggered,
        "floor should decay so soft speech triggers again"
    );
}
