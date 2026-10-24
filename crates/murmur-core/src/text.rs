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
    fn trailing_space() {
        assert_eq!(with_trailing_space("Done."), "Done. ");
        assert_eq!(with_trailing_space("  "), "");
    }
}
