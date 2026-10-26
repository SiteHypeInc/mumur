//! Small string tools shared by the dictionary matcher and the edit learner.

/// Levenshtein distance over chars.
pub fn edit_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    if a.is_empty() {
        return b.len();
    }
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0; b.len() + 1];
    for (i, ca) in a.iter().enumerate() {
        cur[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let sub = prev[j] + usize::from(ca != cb);
            cur[j + 1] = sub.min(prev[j + 1] + 1).min(cur[j] + 1);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}

/// Distance divided by the longer length: 0.0 identical, 1.0 nothing shared.
pub fn normalized_distance(a: &str, b: &str) -> f32 {
    let max = a.chars().count().max(b.chars().count());
    if max == 0 {
        return 0.0;
    }
    edit_distance(a, b) as f32 / max as f32
}

/// American Soundex. Returns None for strings with no ASCII letters.
pub fn soundex(s: &str) -> Option<String> {
    fn code(c: char) -> Option<char> {
        match c {
            'b' | 'f' | 'p' | 'v' => Some('1'),
            'c' | 'g' | 'j' | 'k' | 'q' | 's' | 'x' | 'z' => Some('2'),
            'd' | 't' => Some('3'),
            'l' => Some('4'),
            'm' | 'n' => Some('5'),
            'r' => Some('6'),
            _ => None,
        }
    }
    let letters: Vec<char> = s.chars().filter(|c| c.is_ascii_alphabetic()).map(|c| c.to_ascii_lowercase()).collect();
    let first = *letters.first()?;
    let mut out = String::from(first.to_ascii_uppercase());
    let mut last = code(first);
    for &c in &letters[1..] {
        let d = code(c);
        if let Some(digit) = d.filter(|_| d != last) {
            out.push(digit);
            if out.len() == 4 {
                break;
            }
        }
        // 'h' and 'w' don't separate equal codes; vowels do.
        if c != 'h' && c != 'w' {
            last = d;
        }
    }
    while out.len() < 4 {
        out.push('0');
    }
    Some(out)
}

/// Lowercase, keep only letters and digits. "Charge-Bee" -> "chargebee".
pub fn match_key(s: &str) -> String {
    s.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect()
}

/// Split a word into (leading punctuation, core, trailing punctuation).
pub fn split_punct(word: &str) -> (&str, &str, &str) {
    let start = word.find(|c: char| c.is_alphanumeric()).unwrap_or(word.len());
    let end = word.rfind(|c: char| c.is_alphanumeric()).map(|i| i + word[i..].chars().next().unwrap().len_utf8()).unwrap_or(start);
    (&word[..start], &word[start..end], &word[end..])
}

/// Everyday words that are never vocabulary. Matching "there" to a dictionary
/// term "Theia" would be a disaster, so these only ever get case fixes.
pub fn is_common_word(w: &str) -> bool {
    const COMMON: &[&str] = &[
        "the", "and", "for", "are", "but", "not", "you", "all", "any", "can", "had", "her", "was",
        "one", "our", "out", "day", "get", "has", "him", "his", "how", "man", "new", "now", "old",
        "see", "two", "way", "who", "boy", "did", "its", "let", "put", "say", "she", "too", "use",
        "that", "with", "have", "this", "will", "your", "from", "they", "know", "want", "been",
        "good", "much", "some", "time", "very", "when", "come", "here", "just", "like", "long",
        "make", "many", "more", "only", "over", "such", "take", "than", "them", "well", "were",
        "what", "there", "their", "where", "which", "while", "would", "could", "should", "about",
        "after", "again", "being", "every", "first", "great", "other", "right", "still", "think",
        "those", "three", "today", "under", "until", "maybe", "really", "thing", "things", "send",
        "then", "also", "back", "even", "into", "most", "need", "said", "same", "tell", "work",
        "year", "look", "call", "next", "last", "find", "give", "made", "part", "place", "sure",
        "these", "going", "doing", "because", "before", "through", "people", "please", "thanks",
        "okay", "yeah", "yes", "meeting", "email", "message", "tomorrow", "yesterday",
        "monday", "tuesday", "wednesday", "thursday", "friday", "saturday", "sunday",
        "january", "february", "march", "april", "june", "july", "august", "september",
        "october", "november", "december", "morning", "afternoon", "evening", "tonight", "week",
    ];
    COMMON.contains(&w.to_lowercase().as_str())
}

/// Free, instant cleanup for when no LLM runs (no key, offline, short
/// dictation, or the LLM failed): drop filler sounds and stutters, fix the
/// pronoun "I", and capitalize sentence starts. Deliberately conservative:
/// words like "like" or "so" can carry meaning and are left alone.
/// `capitalize` is off for code/terminal targets, where "git status" must stay lowercase.
pub fn tidy(text: &str, capitalize: bool) -> String {
    const FILLERS: &[&str] = &["um", "umm", "uh", "uhh", "uhm", "erm", "er", "ah", "hmm", "mm", "mhm"];
    let mut words: Vec<String> = Vec::new();
    for raw in text.split_whitespace() {
        let (lead, core, trail) = split_punct(raw);
        let lower = core.to_lowercase();
        if FILLERS.contains(&lower.as_str()) {
            // Keep sentence-ending punctuation the filler was carrying.
            if let (Some(prev), true) = (words.last_mut(), trail.contains(['.', '?', '!'])) {
                let p = prev.trim_end_matches(',').to_owned();
                *prev = format!("{p}{}", trail.trim_start_matches(','));
            }
            continue;
        }
        // "the the" -> "the" (but "had had" and "that that" are real English).
        if let Some(prev) = words.last() {
            let (_, pcore, ptrail) = split_punct(prev);
            if ptrail.is_empty() && lead.is_empty() && pcore.eq_ignore_ascii_case(core) && !core.is_empty()
                && !matches!(lower.as_str(), "had" | "that" | "is" | "very" | "really" | "no" | "so")
            {
                words.pop();
            }
        }
        let fixed_core = match lower.as_str() {
            "i" if capitalize => "I".to_owned(),
            "i'm" | "i’m" => "I'm".to_owned(),
            "i'll" | "i’ll" => "I'll".to_owned(),
            "i've" | "i’ve" => "I've".to_owned(),
            "i'd" | "i’d" => "I'd".to_owned(),
            _ => core.to_owned(),
        };
        words.push(format!("{lead}{fixed_core}{trail}"));
    }
    let mut out = String::with_capacity(text.len());
    let mut cap_next = true;
    for (i, w) in words.iter().enumerate() {
        if i > 0 {
            out.push(' ');
        }
        let mut chars = w.chars();
        if cap_next && capitalize {
            // Capitalize the first letter, skipping leading quotes/brackets.
            let mut done = false;
            for c in chars.by_ref() {
                if !done && c.is_alphabetic() {
                    out.extend(c.to_uppercase());
                    done = true;
                } else {
                    out.push(c);
                }
            }
        } else {
            out.push_str(w);
        }
        cap_next = w.ends_with(['.', '?', '!']);
    }
    // A leftover leading comma from a removed filler ("Um, so" -> ", so").
    out.trim_start_matches([',', ' ']).to_owned()
}

/// Add a trailing space when the text ends in sentence punctuation, so the next
/// dictation doesn't glue onto this one ("Done.Next" -> "Done. Next").
pub fn with_trailing_space(text: &str) -> String {
    let t = text.trim_end();
    if t.is_empty() || t.ends_with('\n') {
        return t.to_owned();
    }
    format!("{t} ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distances() {
        assert_eq!(edit_distance("kitten", "sitting"), 3);
        assert_eq!(edit_distance("", "abc"), 3);
        assert!((normalized_distance("abc", "abc")).abs() < f32::EPSILON);
    }

    #[test]
    fn soundex_codes() {
        assert_eq!(soundex("Robert").as_deref(), Some("R163"));
        assert_eq!(soundex("Rupert").as_deref(), Some("R163"));
        assert_eq!(soundex("Ashcraft").as_deref(), Some("A261"));
        assert_eq!(soundex("Tymczak").as_deref(), Some("T522"));
        assert_eq!(soundex("123"), None);
    }

    #[test]
    fn punct_split() {
        assert_eq!(split_punct("(hello),"), ("(", "hello", "),"));
        assert_eq!(split_punct("Sinéad."), ("", "Sinéad", "."));
        assert_eq!(split_punct("..."), ("...", "", ""));
    }

    #[test]
    fn tidy_cleans_without_changing_meaning() {
        assert_eq!(tidy("um so i think the the deploy is uh ready", true), "So I think the deploy is ready");
        assert_eq!(tidy("Um, can you check it? uh i'm not sure.", true), "Can you check it? I'm not sure.");
        assert_eq!(tidy("we had had enough. that that was it", true), "We had had enough. That that was it");
        assert_eq!(tidy("I like it, like a lot", true), "I like it, like a lot");
        assert_eq!(tidy("ship it um.", true), "Ship it.");
        assert_eq!(tidy("", true), "");
        assert_eq!(tidy("um git status i guess", false), "git status i guess");
    }

    #[test]
    fn trailing_space() {
        assert_eq!(with_trailing_space("Done."), "Done. ");
        assert_eq!(with_trailing_space("  "), "");
    }
}
