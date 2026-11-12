//! Small text helpers shared by the checker, citations, retrieval and the demo provider.

use regex::Regex;
use std::collections::BTreeSet;
use std::sync::LazyLock;

pub static CITATION: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\[\[E(\d+)\]\]").unwrap());

const STOPWORDS: &[&str] = &[
    "about", "above", "after", "again", "against", "all", "also", "and", "any", "are", "because", "been",
    "before", "being", "below", "between", "both", "but", "can", "could", "did", "does", "doing", "down",
    "during", "each", "even", "every", "few", "for", "from", "further", "get", "gets", "had", "has", "have",
    "having", "her", "here", "hers", "him", "his", "how", "into", "its", "it's", "just", "like", "make",
    "makes", "more", "most", "much", "must", "not", "now", "off", "once", "only", "other", "our", "ours",
    "out", "over", "own", "same", "she", "should", "some", "such", "than", "that", "the", "their", "theirs",
    "them", "then", "there", "these", "they", "this", "those", "through", "too", "under", "until", "very",
    "was", "way", "were", "what", "when", "where", "which", "while", "who", "whom", "why", "will", "with",
    "would", "you", "your", "yours", "we", "us", "a", "an", "of", "to", "in", "on", "is", "it", "be", "as",
    "at", "by", "or", "if", "so", "do", "no", "yes", "one", "two", "use", "uses", "used", "using", "new",
    "many", "people", "really", "thing", "things", "want", "need", "needs", "lot", "lots", "well", "good",
];

pub fn is_stopword(word: &str) -> bool {
    STOPWORDS.contains(&word)
}

/// Lower-cased content words of at least four letters.
pub fn significant_words(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric() && c != '\'')
        .map(|w| w.trim_matches('\'').to_lowercase())
        .filter(|w| w.chars().count() >= 4 && !is_stopword(w))
        .collect()
}

/// A crude stem: the first five characters. Good enough for overlap scoring.
pub fn stem(word: &str) -> String {
    word.chars().take(5).collect()
}

pub fn stems(text: &str) -> BTreeSet<String> {
    significant_words(text).iter().map(|w| stem(w)).collect()
}

/// Share of `a`'s stems that also appear in `b`, from 0 to 1.
pub fn overlap(a: &str, b: &str) -> f64 {
    let sa = stems(a);
    if sa.is_empty() {
        return 0.0;
    }
    let sb = stems(b);
    sa.intersection(&sb).count() as f64 / sa.len() as f64
}

pub fn slugify(text: &str) -> String {
    let mut out = String::new();
    let mut dash = false;
    for c in text.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
            dash = false;
        } else if !dash && !out.is_empty() {
            out.push('-');
            dash = true;
        }
    }
    let out = out.trim_end_matches('-').to_string();
    let truncated: String = out.chars().take(80).collect();
    let truncated = truncated.trim_end_matches('-').to_string();
    if truncated.is_empty() {
        "post".into()
    } else {
        truncated
    }
}

/// Splits prose into sentences on `.`, `!` and `?` followed by whitespace.
pub fn sentences(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let chars: Vec<char> = text.chars().collect();
    for (i, c) in chars.iter().enumerate() {
        current.push(*c);
        let end = matches!(c, '.' | '!' | '?') && chars.get(i + 1).map(|n| n.is_whitespace()).unwrap_or(true);
        if end || *c == '\n' {
            let s = current.trim().to_string();
            if !s.is_empty() {
                out.push(s);
            }
            current.clear();
        }
    }
    let s = current.trim().to_string();
    if !s.is_empty() {
        out.push(s);
    }
    out
}

pub fn word_count(text: &str) -> usize {
    text.split_whitespace().count()
}

/// Removes `[[E12]]` citation markers and tidies the spaces they leave behind.
pub fn strip_citations(markdown: &str) -> String {
    let stripped = CITATION.replace_all(markdown, "");
    let tidy = Regex::new(r"[ \t]+([.,;:!?])").unwrap();
    let doubled = Regex::new(r"[ \t]{2,}").unwrap();
    let trailing = Regex::new(r"[ \t]+\n").unwrap();
    let s = tidy.replace_all(&stripped, "$1");
    let s = doubled.replace_all(&s, " ");
    trailing.replace_all(&s, "\n").trim_end().to_string() + "\n"
}

/// Entry numbers cited in a body, in order of first appearance.
pub fn cited_nums(markdown: &str) -> Vec<i32> {
    let mut seen = Vec::new();
    for cap in CITATION.captures_iter(markdown) {
        if let Ok(n) = cap[1].parse::<i32>() {
            if !seen.contains(&n) {
                seen.push(n);
            }
        }
    }
    seen
}

pub fn truncate_words(text: &str, max: usize) -> String {
    let words: Vec<&str> = text.split_whitespace().collect();
    if words.len() <= max {
        text.trim().to_string()
    } else {
        words[..max].join(" ")
    }
}

pub fn title_case(text: &str) -> String {
    text.split_whitespace()
        .map(|w| {
            let mut c = w.chars();
            match c.next() {
                Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_citations_cleanly() {
        assert_eq!(strip_citations("Fast setup [[E3]]. Next [[E4]][[E5]]\n"), "Fast setup. Next\n");
    }

    #[test]
    fn finds_cited_numbers_once() {
        assert_eq!(cited_nums("a [[E2]] b [[E10]] c [[E2]]"), vec![2, 10]);
    }

    #[test]
    fn slugs() {
        assert_eq!(slugify("How to Choose a CRM: A Buyer's Guide!"), "how-to-choose-a-crm-a-buyer-s-guide");
    }
}
