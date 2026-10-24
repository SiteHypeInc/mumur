//! Deterministic vocabulary fixes that run before and after the LLM.
//!
//! Two layers:
//! * `replacements`: exact phrase rewrites the user asked for ("my email" -> "john@x.com").
//! * `terms`: spellings to snap near-misses to ("Shinade" -> "Sinéad", "charge bee" -> "ChargeBee").
//!
//! Doing this in code (not only in the prompt) makes learned words stick even
//! when the LLM step is off, slow, or ignores the vocabulary list.

use serde::{Deserialize, Serialize};

use crate::text::{is_common_word, match_key, normalized_distance, soundex, split_punct};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Replacement {
    pub from: String,
    pub to: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Dictionary {
    pub terms: Vec<String>,
    pub replacements: Vec<Replacement>,
}

/// Lower is stricter. 0.2 accepts one wrong letter in five.
pub const DEFAULT_THRESHOLD: f32 = 0.2;
/// Sounding alike buys a little slack, but only for longer words: "cloud" and
/// "Claude" share a Soundex code, and nobody wants every cloud to become Claude.
const PHONETIC_BONUS: f32 = 0.1;
const PHONETIC_MIN_LEN: usize = 6;
const MAX_NGRAM: usize = 3;

impl Dictionary {
    pub fn add_terms<I: IntoIterator<Item = String>>(&mut self, words: I) -> Vec<String> {
        let mut added = Vec::new();
        for w in words {
            let w = w.trim().to_owned();
            if w.is_empty() || self.terms.iter().any(|t| t.eq_ignore_ascii_case(&w)) {
                continue;
            }
            self.terms.push(w.clone());
            added.push(w);
        }
        added
    }

    pub fn apply(&self, text: &str, threshold: f32) -> String {
        let replaced = apply_replacements(text, &self.replacements);
        snap_terms(&replaced, &self.terms, threshold)
    }
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '\'' || c == '’'
}

/// Case-insensitive whole-phrase replacement.
pub fn apply_replacements(text: &str, reps: &[Replacement]) -> String {
    let mut out = text.to_owned();
    for r in reps {
        let from = r.from.trim();
        if from.is_empty() {
            continue;
        }
        let lower_out = out.to_lowercase();
        let lower_from = from.to_lowercase();
        // Lowercasing can change byte lengths for some scripts; only do index
        // math when it didn't.
        if lower_out.len() != out.len() {
            continue;
        }
        let mut result = String::with_capacity(out.len());
        let mut i = 0;
        while let Some(pos) = lower_out[i..].find(&lower_from) {
            let start = i + pos;
            let end = start + lower_from.len();
            let before_ok = out[..start].chars().next_back().is_none_or(|c| !is_word_char(c));
            let after_ok = out[end..].chars().next().is_none_or(|c| !is_word_char(c));
            result.push_str(&out[i..start]);
            if before_ok && after_ok {
                result.push_str(&r.to);
            } else {
                result.push_str(&out[start..end]);
            }
            i = end;
        }
        result.push_str(&out[i..]);
        out = result;
    }
    out
}

struct Term<'a> {
    text: &'a str,
    key: String,
    phonetic: Option<String>,
}

fn score(candidate: &str, term: &Term) -> Option<f32> {
    let (cl, tl) = (candidate.chars().count(), term.key.chars().count());
    if cl == 0 || tl == 0 {
        return None;
    }
    let longer = cl.max(tl);
    let diff = cl.abs_diff(tl);
    if diff > (longer / 4).max(2) {
        return None;
    }
    let mut s = normalized_distance(candidate, &term.key);
    let alpha = candidate.chars().all(|c| c.is_ascii_alphabetic()) && term.key.chars().all(|c| c.is_ascii_alphabetic());
    if alpha && cl >= PHONETIC_MIN_LEN && soundex(candidate).is_some() && soundex(candidate) == term.phonetic {
        s = (s - PHONETIC_BONUS).max(0.0001);
    }
    Some(s)
}

/// Snap 1–3 word spans to the closest vocabulary term, preserving surrounding
/// punctuation. Terms are written exactly as stored (that's the point: casing
/// like "iPhone" or "GitHub" is part of the term).
pub fn snap_terms(text: &str, terms: &[String], threshold: f32) -> String {
    let terms: Vec<Term> = terms
        .iter()
        .map(|t| t.trim())
        .filter(|t| !t.is_empty())
        .map(|t| Term { text: t, key: match_key(t), phonetic: soundex(&match_key(t)) })
        .filter(|t| t.key.chars().count() >= 2)
        .collect();
    if terms.is_empty() {
        return text.to_owned();
    }

    // Tokenize on whitespace but keep the separators so layout survives.
    let mut words: Vec<&str> = Vec::new();
    let mut seps: Vec<&str> = Vec::new(); // seps[i] precedes words[i]
    let mut rest = text;
    loop {
        let ws_len = rest.len() - rest.trim_start().len();
        let (sep, after) = rest.split_at(ws_len);
        if after.is_empty() {
            seps.push(sep);
            break;
        }
        let wlen = after.find(char::is_whitespace).unwrap_or(after.len());
        seps.push(sep);
        words.push(&after[..wlen]);
        rest = &after[wlen..];
    }

    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < words.len() {
        let mut best: Option<(usize, f32, &str)> = None;
        for n in 1..=MAX_NGRAM.min(words.len() - i) {
            let span = &words[i..i + n];
            // A phrase may not run across punctuation inside it.
            if span[..n - 1].iter().any(|w| !split_punct(w).2.is_empty())
                || span[1..].iter().any(|w| !split_punct(w).0.is_empty())
            {
                break;
            }
            let joined: String = span.iter().map(|w| match_key(split_punct(w).1)).collect();
            if joined.chars().count() < 2 {
                continue;
            }
            let single_common = n == 1 && is_common_word(&joined);
            for term in &terms {
                let Some(s) = score(&joined, term) else { continue };
                if single_common && s > 0.0 {
                    continue;
                }
                // Short words need an exact key match; fuzzy 3-letter matches are noise.
                if s > 0.0 && joined.chars().count() < 4 {
                    continue;
                }
                if s < threshold && best.is_none_or(|(_, bs, _)| s < bs) {
                    best = Some((n, s, term.text));
                }
            }
        }

        out.push_str(seps[i]);
        match best {
            Some((n, _, term)) => {
                let lead = split_punct(words[i]).0;
                let trail = split_punct(words[i + n - 1]).2;
                out.push_str(lead);
                out.push_str(term);
                out.push_str(trail);
                i += n;
            }
            None => {
                out.push_str(words[i]);
                i += 1;
            }
        }
    }
    out.push_str(seps[words.len()]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snap(text: &str, terms: &[&str]) -> String {
        let terms: Vec<String> = terms.iter().map(|s| s.to_string()).collect();
        snap_terms(text, &terms, DEFAULT_THRESHOLD)
    }

    #[test]
    fn fixes_case_and_near_misses() {
        assert_eq!(snap("push it to github.", &["GitHub"]), "push it to GitHub.");
        assert_eq!(snap("ask Shinead about it", &["Sinead"]), "ask Sinead about it");
        assert_eq!(snap("deploy with kubernetis today", &["Kubernetes"]), "deploy with Kubernetes today");
    }

    #[test]
    fn joins_split_words() {
        assert_eq!(snap("we use charge bee for billing", &["ChargeBee"]), "we use ChargeBee for billing");
        assert_eq!(snap("open the post hog dashboard", &["PostHog"]), "open the PostHog dashboard");
    }

    #[test]
    fn leaves_common_and_unrelated_words_alone() {
        assert_eq!(snap("put it there", &["Theia"]), "put it there");
        assert_eq!(snap("the cat sat", &["Cato"]), "the cat sat");
        assert_eq!(snap("hello world", &["Kubernetes"]), "hello world");
        assert_eq!(snap("store it in the cloud", &["Claude"]), "store it in the cloud");
    }

    #[test]
    fn keeps_punctuation_and_layout() {
        assert_eq!(snap("(github), ok\n\nnext", &["GitHub"]), "(GitHub), ok\n\nnext");
        assert_eq!(snap("  lead and trail  ", &["Zed"]), "  lead and trail  ");
    }

    #[test]
    fn does_not_join_across_punctuation() {
        assert_eq!(snap("charge. bee", &["ChargeBee"]), "charge. bee");
    }

    #[test]
    fn replacements_are_whole_phrase_and_case_insensitive() {
        let reps = vec![Replacement { from: "my email".into(), to: "john@example.com".into() }];
        assert_eq!(apply_replacements("Send to My Email please", &reps), "Send to john@example.com please");
        assert_eq!(apply_replacements("my emails", &reps), "my emails");
    }

    #[test]
    fn add_terms_dedupes() {
        let mut d = Dictionary::default();
        assert_eq!(d.add_terms(vec!["GitHub".into(), "github".into(), " ".into()]), vec!["GitHub".to_string()]);
        assert_eq!(d.terms, vec!["GitHub".to_string()]);
    }
}
