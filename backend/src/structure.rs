//! The page-structure checker: rules that make pages answer first and quote well in AI
//! answers. A pure function from Markdown to findings; no AI involved.

use crate::text::{strip_citations, word_count};
use serde::Serialize;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Finding {
    pub rule: &'static str,
    pub message: String,
    /// Index of the block (paragraph, heading, table, list) the finding is about.
    pub block: usize,
    /// Start of the offending text, to help locate it.
    pub excerpt: String,
}

#[derive(Debug, PartialEq, Eq)]
enum Block {
    Heading(String),
    Paragraph(String),
    Table,
    List(String),
    Code,
    Other,
}

fn blocks(markdown: &str) -> Vec<Block> {
    let mut out = Vec::new();
    let mut lines = markdown.lines().peekable();
    let mut buf: Vec<String> = Vec::new();
    let flush = |buf: &mut Vec<String>, out: &mut Vec<Block>| {
        if buf.is_empty() {
            return;
        }
        let text = buf.join(" ").trim().to_string();
        let first = buf[0].trim_start();
        let block = if first.starts_with('|') {
            Block::Table
        } else if first.starts_with("- ") || first.starts_with("* ") || first.starts_with("+ ") || first.split_once(". ").map(|(n, _)| n.chars().all(|c| c.is_ascii_digit())).unwrap_or(false) {
            Block::List(text)
        } else if first.starts_with('>') || first.starts_with("<") || first.starts_with("---") {
            Block::Other
        } else {
            Block::Paragraph(text)
        };
        out.push(block);
        buf.clear();
    };
    while let Some(line) = lines.next() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") {
            flush(&mut buf, &mut out);
            for l in lines.by_ref() {
                if l.trim_start().starts_with("```") {
                    break;
                }
            }
            out.push(Block::Code);
        } else if trimmed.starts_with('#') {
            flush(&mut buf, &mut out);
            out.push(Block::Heading(trimmed.trim_start_matches('#').trim().to_string()));
        } else if trimmed.is_empty() {
            flush(&mut buf, &mut out);
        } else {
            buf.push(line.to_string());
        }
    }
    flush(&mut buf, &mut out);
    out
}

const WARM_UPS: &[&str] = &[
    "in today's", "in this article", "in this post", "in this guide", "in this blog", "have you ever", "welcome to",
    "let's dive", "let's take a look", "let's explore", "when it comes to", "it's no secret", "in the world of",
    "in the fast-paced", "in an ever", "imagine ", "are you looking", "if you're like most", "before we dive",
    "we've all been there",
];

/// Openers that only make sense after the paragraph before.
const DEPENDENT_OPENERS: &[&str] = &[
    "as mentioned", "as noted", "as we saw", "as we discussed", "as discussed", "as stated", "as shown above",
    "as explained", "as described above", "the above", "mentioned above", "this ", "that ", "these ", "those ",
    "however,", "furthermore", "moreover", "additionally", "also,", "similarly,", "in addition", "on the other hand",
    "the former", "the latter", "it also", "they also", "besides,",
];

const LONG_SECTION_WORDS: usize = 300;

fn excerpt(text: &str) -> String {
    let words: Vec<&str> = text.split_whitespace().take(10).collect();
    words.join(" ")
}

pub fn check(markdown: &str, content_type: &str) -> Vec<Finding> {
    let clean = strip_citations(markdown);
    let blocks = blocks(&clean);
    let mut findings = Vec::new();

    // Rule 1: the intro must answer straight away.
    match blocks.iter().enumerate().find(|(_, b)| !matches!(b, Block::Other)) {
        Some((i, Block::Paragraph(text))) => {
            let lower = text.to_lowercase();
            if WARM_UPS.iter().any(|w| lower.starts_with(w) || lower.contains(&format!(". {w}"))) || text.ends_with('?') {
                findings.push(Finding {
                    rule: "intro_answer_first",
                    message: "The intro warms up instead of answering. Open with the answer itself.".into(),
                    block: i,
                    excerpt: excerpt(text),
                });
            }
        }
        Some((i, _)) => findings.push(Finding {
            rule: "intro_answer_first",
            message: "The page starts without an answering paragraph. Put a direct answer before the first heading.".into(),
            block: i,
            excerpt: String::new(),
        }),
        None => findings.push(Finding {
            rule: "intro_answer_first",
            message: "The page is empty.".into(),
            block: 0,
            excerpt: String::new(),
        }),
    }

    // Rule 2: every paragraph must stand alone.
    for (i, b) in blocks.iter().enumerate() {
        if let Block::Paragraph(text) = b {
            let lower = text.to_lowercase();
            if DEPENDENT_OPENERS.iter().any(|o| lower.starts_with(o)) || lower.contains("as mentioned above") || lower.contains("see above") {
                findings.push(Finding {
                    rule: "paragraph_stands_alone",
                    message: "This paragraph depends on the one before it. Name the subject so it can be quoted on its own.".into(),
                    block: i,
                    excerpt: excerpt(text),
                });
            }
        }
    }

    // Rule 3: long sections need headings.
    let mut words = 0;
    let mut section_start = 0;
    let mut flagged = false;
    for (i, b) in blocks.iter().enumerate() {
        match b {
            Block::Heading(_) => {
                words = 0;
                section_start = i;
                flagged = false;
            }
            Block::Paragraph(t) | Block::List(t) => {
                words += word_count(t);
                if words > LONG_SECTION_WORDS && !flagged {
                    findings.push(Finding {
                        rule: "long_section",
                        message: format!("This section runs past {LONG_SECTION_WORDS} words without a heading. Break it up so it can be scanned."),
                        block: section_start,
                        excerpt: excerpt(t),
                    });
                    flagged = true;
                }
            }
            _ => {}
        }
    }

    // Rule 4: alternatives and comparison pages need a comparison table.
    if matches!(content_type, "alternatives" | "comparison") && !blocks.iter().any(|b| *b == Block::Table) {
        findings.push(Finding {
            rule: "comparison_table",
            message: "Alternatives and comparison pages need a comparison table, for example \"best for\" by option.".into(),
            block: 0,
            excerpt: String::new(),
        });
    }
    findings
}

/// Share of paragraphs that carry at least one citation, from 0 to 1.
pub fn citation_coverage(markdown: &str) -> f64 {
    let paragraphs: Vec<String> = blocks(markdown)
        .into_iter()
        .filter_map(|b| match b {
            Block::Paragraph(t) | Block::List(t) => Some(t),
            _ => None,
        })
        .collect();
    if paragraphs.is_empty() {
        return 0.0;
    }
    let cited = paragraphs.iter().filter(|p| crate::text::CITATION.is_match(p)).count();
    cited as f64 / paragraphs.len() as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rules(md: &str, kind: &str) -> Vec<&'static str> {
        check(md, kind).into_iter().map(|f| f.rule).collect()
    }

    #[test]
    fn a_direct_answer_passes() {
        let md = "A CRM keeps every customer conversation in one place [[E1]].\n\n## Why it matters\n\nSales teams lose deals when notes live in inboxes [[E2]].\n";
        assert!(rules(md, "explainer").is_empty());
    }

    #[test]
    fn warm_up_intros_are_flagged() {
        assert_eq!(rules("In today's fast-paced world, sales matter.\n", "explainer"), vec!["intro_answer_first"]);
        assert_eq!(rules("Have you ever lost a deal?\n", "explainer"), vec!["intro_answer_first"]);
    }

    #[test]
    fn a_heading_first_page_is_flagged() {
        assert!(rules("## Intro\n\nText here that answers.\n", "explainer").contains(&"intro_answer_first"));
    }

    #[test]
    fn dependent_paragraphs_are_flagged() {
        let md = "CRMs centralise notes.\n\nAs mentioned above, this saves time.\n\nThis is why teams switch.\n";
        let f = check(md, "explainer");
        assert_eq!(f.iter().filter(|f| f.rule == "paragraph_stands_alone").count(), 2);
    }

    #[test]
    fn long_sections_are_flagged_once() {
        let para = "word ".repeat(200);
        let md = format!("Answer first.\n\n## Section\n\n{para}\n\n{para}\n\n{para}\n");
        assert_eq!(rules(&md, "explainer"), vec!["long_section"]);
    }

    #[test]
    fn alternatives_need_a_table() {
        assert_eq!(rules("Here are the best options.\n", "alternatives"), vec!["comparison_table"]);
        let with_table = "Here are the best options.\n\n| Option | Best for |\n| --- | --- |\n| A | B |\n";
        assert!(rules(with_table, "alternatives").is_empty());
    }

    #[test]
    fn citations_are_ignored_when_checking() {
        assert!(rules("[[E1]] A CRM keeps notes together.\n", "explainer").is_empty());
    }

    #[test]
    fn coverage_counts_cited_paragraphs() {
        assert_eq!(citation_coverage("One [[E1]].\n\nTwo.\n"), 0.5);
    }
}
