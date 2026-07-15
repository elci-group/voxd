//! Tiny RMS energy VAD with silence hangover, max-utterance cap, and an
//! adaptive noise floor so steady ambient noise stops triggering STT calls.

/// Samples per 20 ms frame at 16 kHz.
pub const FRAME_SAMPLES: usize = 320;

/// How fast the noise floor tracks quiet frames (per-frame EMA weight).
const FLOOR_ALPHA: f32 = 0.01;

pub struct Vad {
    threshold: f32,
    /// Speech must exceed `noise_floor * margin` (or `threshold`, whichever is
    /// higher) to count as loud.
    margin: f32,
    noise_floor: f32,
    silence_frames_needed: usize,
    max_samples: usize,
    in_speech: bool,
    silence_run: usize,
    buf: Vec<i16>,
    rms_sum: f64,
    rms_count: usize,
}

impl Vad {
    pub fn new(
        threshold: f32,
        silence_ms: u64,
        max_utterance_secs: u64,
        noise_margin: f32,
    ) -> Self {
        let silence_frames_needed = silence_ms.div_ceil(20).max(1) as usize;
        let max_frames = (max_utterance_secs.max(1) * 50) as usize; // 50 frames/s
        Self {
            threshold,
            margin: noise_margin.max(1.0),
            noise_floor: 0.0,
            silence_frames_needed,
            max_samples: max_frames * FRAME_SAMPLES,
            in_speech: false,
            silence_run: 0,
            buf: Vec::new(),
            rms_sum: 0.0,
            rms_count: 0,
        }
    }

    /// Feed one 20 ms frame (≈320 samples). Returns a completed utterance when
    /// enough trailing silence accumulates or the max length is reached.
    pub fn feed(&mut self, frame: &[i16]) -> Option<Vec<i16>> {
        let level = rms(frame);
        let effective = self.threshold.max(self.noise_floor * self.margin);
        let loud = level > effective;
        if !loud {
            // Quiet frame: let the floor drift toward the observed ambient
            // level (decays back to the raw threshold in a silent room).
            self.noise_floor += (level - self.noise_floor) * FLOOR_ALPHA;
        }
        if self.in_speech {
            self.buf.extend_from_slice(frame);
            self.rms_sum += level as f64;
            self.rms_count += 1;
            if loud {
                self.silence_run = 0;
            } else {
                self.silence_run += 1;
            }
            if self.silence_run >= self.silence_frames_needed {
                return self.emit();
            }
            if self.buf.len() >= self.max_samples {
                // Ran to the cap without ever closing on silence: almost
                // certainly sustained ambient noise, not speech. Snap the
                // floor up to the utterance average so it stops
                // re-triggering, then emit this one last time.
                if self.rms_count > 0 {
                    let avg = (self.rms_sum / self.rms_count as f64) as f32;
                    if avg > self.noise_floor {
                        self.noise_floor = avg;
                    }
                }
                return self.emit();
            }
        } else if loud {
            self.in_speech = true;
            self.silence_run = 0;
            self.buf.extend_from_slice(frame);
            self.rms_sum = level as f64;
            self.rms_count = 1;
        }
        None
    }

    fn emit(&mut self) -> Option<Vec<i16>> {
        if self.buf.is_empty() {
            self.in_speech = false;
            self.silence_run = 0;
            return None;
        }
        let utt = std::mem::take(&mut self.buf);
        self.in_speech = false;
        self.silence_run = 0;
        self.rms_sum = 0.0;
        self.rms_count = 0;
        Some(utt)
    }

    pub fn reset(&mut self) {
        self.in_speech = false;
        self.silence_run = 0;
        self.buf.clear();
        self.rms_sum = 0.0;
        self.rms_count = 0;
    }
}

pub fn rms(frame: &[i16]) -> f32 {
    if frame.is_empty() {
        return 0.0;
    }
    let sum: f64 = frame.iter().map(|&s| (s as f64) * (s as f64)).sum();
    ((sum / frame.len() as f64).sqrt() / 32768.0) as f32
}
