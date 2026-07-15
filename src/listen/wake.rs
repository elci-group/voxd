//! Wake-word matching on STT transcripts (normalized, fuzzy).

const GREETINGS: &[&str] = &["hey", "hi", "hello", "ok", "okay"];
const VOX_TAILS: &[&str] = &["t", "tee", "tea", "d", "dee", "the"];

/// Lowercase, drop punctuation, collapse whitespace.
pub fn normalize(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut last_space = false;
    for ch in s.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            last_space = false;
        } else if !last_space {
            out.push(' ');
            last_space = true;
        }
    }
    out.trim().to_string()
}

#[derive(Debug, Clone)]
pub struct WakeMatcher {
    phrase: String,
}

impl WakeMatcher {
    pub fn new(phrase: &str) -> Self {
        Self {
            phrase: phrase.to_string(),
        }
    }

    /// Returns the command text following the wake phrase (possibly empty), or
    /// `None` if the transcript does not start with the wake phrase.
    pub fn check(&self, text: &str) -> Option<String> {
        let ntext = normalize(text);
        let ntokens: Vec<&str> = ntext.split_whitespace().collect();
        let nphrase = normalize(&self.phrase);
        let ptokens: Vec<&str> = nphrase.split_whitespace().collect();
        let consumed = match_consumed(&ntokens, &ptokens, &ntext, &nphrase)?;

        let orig_words: Vec<&str> = text.split_whitespace().collect();
        let cmd = if consumed < orig_words.len() {
            orig_words[consumed..].join(" ")
        } else {
            String::new()
        };
        Some(cmd)
    }
}

fn match_consumed(
    ntokens: &[&str],
    ptokens: &[&str],
    _ntext: &str,
    _nphrase: &str,
) -> Option<usize> {
    // 1) exact phrase prefix, token-wise.
    if !ptokens.is_empty() && ntokens.starts_with(ptokens) {
        return Some(ptokens.len());
    }
    // 2) tolerant: greeting + a token starting with "vox", with an optional
    //    trailing mishear token ("hey vox t", "hey vox dee", ...).
    //    (A squashed/no-space match was considered but rejected: "voxd" is a
    //    prefix of "voxdee", causing false short matches.)
    if ntokens.len() >= 2 && GREETINGS.contains(&ntokens[0]) {
        let second = ntokens[1];
        if second.starts_with("vox") {
            if second == "vox"
                && ntokens
                    .get(2)
                    .map(|t| VOX_TAILS.contains(t))
                    .unwrap_or(false)
            {
                return Some(3);
            }
            return Some(2);
        }
    }
    None
}
