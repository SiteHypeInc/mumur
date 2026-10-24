//! Learn vocabulary from the user's own fixes.
//!
//! After a paste, the OS layer watches the target text field. When the user
//! edits what we typed, we diff our text against the field and keep word
//! substitutions that look like recognition errors (similar spelling, or a
//! casing fix like "github" -> "GitHub"). Content rewrites are ignored.

use crate::text::{is_common_word, match_key, normalized_distance, soundex, split_punct};

/// How different a correction may be from what we typed and still count as a
/// misheard word rather than a different word.
const MAX_SPELLING_DISTANCE: f32 = 0.6;

fn words(s: &str) -> Vec<&str> {
    s.split_whitespace()
        .map(|w| split_punct(w).1)
        .filter(|w| !w.is_empty())
        .collect()
}

fn eq(a: &str, b: &str) -> bool {
    a.to_lowercase() == b.to_lowercase()
}

/// Locate the slice of `field` that corresponds to what we pasted. The field
/// usually holds other text too (an email thread, a chat draft).
fn edited_region<'a>(pasted: &[&'a str], field: &[&'a str]) -> Vec<&'a str> {
    if field.len() <= pasted.len() + pasted.len() / 2 {
        return field.to_vec();
    }
    let w = pasted.len();
    let mut best = (0usize, 0usize);
    for start in 0..=field.len() - w {
        let hits = (0..w).filter(|&k| eq(field[start + k], pasted[k])).count();
        if hits > best.1 {
            best = (start, hits);
        }
    }
    // Under 40% overlap we can't tell which part of the field is ours.
    if best.1 * 10 < w * 4 {
        return Vec::new();
    }
    field[best.0..best.0 + w].to_vec()
}

#[derive(Debug, PartialEq)]
enum Op<'a> {
    Keep,
    Del(&'a str),
    Ins(&'a str),
}

fn align<'a>(a: &[&'a str], b: &[&'a str]) -> Vec<Op<'a>> {
    let (m, n) = (a.len(), b.len());
    let mut dp = vec![vec![0u16; n + 1]; m + 1];
    for i in (0..m).rev() {
        for j in (0..n).rev() {
            dp[i][j] = if eq(a[i], b[j]) { dp[i + 1][j + 1] + 1 } else { dp[i + 1][j].max(dp[i][j + 1]) };
        }
    }
    let (mut i, mut j) = (0, 0);
    let mut ops = Vec::new();
    while i < m || j < n {
        if i < m && j < n && eq(a[i], b[j]) {
            // Same word, different case is still a substitution we may learn from.
            if a[i] != b[j] {
                ops.push(Op::Del(a[i]));
                ops.push(Op::Ins(b[j]));
            } else {
                ops.push(Op::Keep);
            }
            i += 1;
            j += 1;
        } else if j < n && (i == m || dp[i][j + 1] >= dp[i + 1][j]) {
            ops.push(Op::Ins(b[j]));
            j += 1;
        } else {
            ops.push(Op::Del(a[i]));
            i += 1;
        }
    }
    ops
}

/// Group consecutive deletions + insertions into (removed, added) phrases.
fn substitutions<'a>(ops: &[Op<'a>]) -> Vec<(Vec<&'a str>, Vec<&'a str>)> {
    let mut subs = Vec::new();
    let mut k = 0;
    while k < ops.len() {
        let (mut del, mut ins) = (Vec::new(), Vec::new());
        while k < ops.len() && !matches!(ops[k], Op::Keep) {
            match ops[k] {
                Op::Del(w) => del.push(w),
                Op::Ins(w) => ins.push(w),
                Op::Keep => unreachable!(),
            }
            k += 1;
        }
        if !del.is_empty() && !ins.is_empty() {
            subs.push((del, ins));
        }
        k += 1;
    }
    subs
}

/// Returns new vocabulary terms (as the user typed them) not already in `known`.
pub fn learn_from_edit(pasted: &str, field: &str, known: &[String]) -> Vec<String> {
    if pasted.trim().is_empty() || pasted == field {
        return Vec::new();
    }
    let ours = words(pasted);
    let region = edited_region(&ours, &words(field));
    if region.is_empty() || ours.is_empty() {
        return Vec::new();
    }
    let subs = substitutions(&align(&ours, &region));
    // Many changes means the user rewrote it; there's nothing to learn about the recognizer.
    let changed: usize = subs.iter().map(|(d, _)| d.len()).sum();
    if changed * 2 > ours.len().max(2) {
        return Vec::new();
    }

    let mut learned: Vec<String> = Vec::new();
    for (del, ins) in subs {
        // One spoken thing -> one written thing; allow "charge bee" -> "ChargeBee".
        if del.len() > 3 || ins.len() > 2 {
            continue;
        }
        let before = del.join(" ");
        let after = ins.join(" ");
        let (kb, ka) = (match_key(&before), match_key(&after));
        if ka.chars().count() < 3 || is_common_word(&after) {
            continue;
        }
        let case_fix = kb == ka && after.chars().any(char::is_uppercase);
        let close = normalized_distance(&kb, &ka) <= MAX_SPELLING_DISTANCE;
        let sounds_same = soundex(&kb).is_some() && soundex(&kb) == soundex(&ka);
        if !(case_fix || (kb != ka && (close || sounds_same))) {
            continue;
        }
        // A capital at the start of a word that merely starts a sentence isn't vocabulary.
        if case_fix && !after.chars().skip(1).any(char::is_uppercase) && !is_capitalized_mid_sentence(&before) {
            continue;
        }
        let dup = known.iter().chain(learned.iter()).any(|k| k.eq_ignore_ascii_case(&after));
        if !dup {
            learned.push(after);
        }
    }
    learned
}

// Without the original sentence position we can't prove a capital is
// meaningful, so single-capital case fixes ("paris" -> "Paris") are accepted
// only for words that aren't ordinary lowercase English.
fn is_capitalized_mid_sentence(before: &str) -> bool {
    !is_common_word(before)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn learn(p: &str, f: &str) -> Vec<String> {
        learn_from_edit(p, f, &[])
    }

    #[test]
    fn learns_misheard_name() {
        assert_eq!(learn("Can you ask Shinade to review it?", "Can you ask Sinéad to review it?"), vec!["Sinéad"]);
    }

    #[test]
    fn learns_casing_and_joined_words() {
        assert_eq!(learn("I pushed it to github yesterday.", "I pushed it to GitHub yesterday."), vec!["GitHub"]);
        assert_eq!(learn("We bill through charge bee now.", "We bill through ChargeBee now."), vec!["ChargeBee"]);
    }

    #[test]
    fn ignores_content_edits_and_rewrites() {
        assert!(learn("Send it on Thursday.", "Send it on Friday.").is_empty());
        assert!(learn("Let's meet at noon to discuss.", "Totally different sentence here now.").is_empty());
        assert!(learn("same text", "same text").is_empty());
    }

    #[test]
    fn finds_our_text_inside_a_longer_field() {
        let field = "Hi team,\n\nQuick update: the Kubernetes cluster is back.\n\nThanks";
        assert_eq!(learn("the kubernetis cluster is back.", field), vec!["Kubernetes"]);
    }

    #[test]
    fn skips_known_terms() {
        assert!(learn_from_edit("push to github", "push to GitHub", &["github".into()]).is_empty());
    }
}
