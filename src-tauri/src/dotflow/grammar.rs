//! DotFlow — offline grammar & spelling cleanup via Harper (github.com/automattic/harper).
//!
//! Fully local: no network, no API key, milliseconds to lint. This is the mid tier of the cleanup hotkey —
//! better than the mechanical [`super::cleanup`] pass, and used when no post-process LLM is configured. It
//! applies Harper's concrete corrections (spelling, agreement, grammar, capitalization, punctuation, …) but
//! skips subjective [`LintKind::Enhancement`] rewrites and any lint that offers no replacement, so it only
//! ever makes safe, confident edits.

use harper_core::linting::{LintGroup, LintKind, Linter, Suggestion};
use harper_core::spell::FstDictionary;
use harper_core::{Dialect, Document, Span};
use serde::Serialize;
use specta::Type;

/// One reviewable issue Harper found — the data a Grammarly-style review panel needs to underline the span
/// and offer click-to-fix replacements. `start`/`end` are CHARACTER offsets into the analyzed text.
#[derive(Debug, Clone, Serialize, Type)]
pub struct TextSuggestion {
    pub start: usize,
    pub end: usize,
    /// Lint category (e.g. "Spelling", "Grammar", "Capitalization"), for coloring/grouping in the UI.
    pub kind: String,
    pub message: String,
    /// The offending text (the substring at `start..end`).
    pub original: String,
    /// Suggested replacements, best-first. An empty string means "remove the offending text".
    pub replacements: Vec<String>,
}

/// Analyze `text` and return Harper's reviewable suggestions (spans + replacements) without changing the
/// text — for the review panel, where the user accepts/rejects each fix. Panic-safe; returns an empty list
/// on blank input or any internal Harper edge case.
pub fn analyze(text: &str) -> Vec<TextSuggestion> {
    if text.trim().is_empty() {
        return Vec::new();
    }
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| analyze_inner(text))).unwrap_or_else(
        |_| {
            log::warn!("grammar::analyze panicked; returning no suggestions");
            Vec::new()
        },
    )
}

fn analyze_inner(text: &str) -> Vec<TextSuggestion> {
    let chars: Vec<char> = text.chars().collect();
    let char_count = chars.len();
    let document = Document::new_plain_english_curated(text);
    let dict = FstDictionary::curated();
    let mut linter = LintGroup::new_curated(dict, Dialect::American);

    linter
        .lint(&document)
        .iter()
        .filter(|l| l.span.start <= l.span.end && l.span.end <= char_count)
        .filter(|l| !matches!(l.lint_kind, LintKind::Enhancement))
        .map(|l| {
            let replacements = l
                .suggestions
                .iter()
                .filter_map(|s| match s {
                    Suggestion::ReplaceWith(cs) => Some(cs.iter().collect::<String>()),
                    Suggestion::Remove => Some(String::new()),
                    // Insertion suggestions don't fit the "replace this span" model the panel uses; skip for v1.
                    Suggestion::InsertAfter(_) => None,
                })
                .collect();
            TextSuggestion {
                start: l.span.start,
                end: l.span.end,
                kind: format!("{:?}", l.lint_kind),
                message: l.message.clone(),
                original: chars[l.span.start..l.span.end].iter().collect(),
                replacements,
            }
        })
        .collect()
}

/// Apply Harper's offline corrections to `text` and return the corrected string. Total + side-effect free
/// (aside from Harper's own cached dictionary). Returns the input unchanged when it's blank, when Harper
/// finds nothing to fix, or if Harper ever panics — so it can NEVER break the caller (the cleanup hotkey).
pub fn harper_cleanup(text: &str) -> String {
    if text.trim().is_empty() {
        return text.to_string();
    }
    // Belt-and-suspenders: a bad span or an internal Harper edge case must not take down the whole action
    // (it runs in a spawned task, where a panic would just vanish and the hotkey would "do nothing").
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| harper_cleanup_inner(text)))
        .unwrap_or_else(|_| {
            log::warn!("harper_cleanup panicked; returning the text unchanged");
            text.to_string()
        })
}

fn harper_cleanup_inner(text: &str) -> String {
    let char_count = text.chars().count();
    let document = Document::new_plain_english_curated(text);
    let dict = FstDictionary::curated();
    let mut linter = LintGroup::new_curated(dict, Dialect::American);
    let lints = linter.lint(&document);

    // Keep only lints that (a) have a span within bounds, (b) offer a concrete replacement, and (c) are
    // actual corrections — not optional style "enhancements" we don't want to auto-apply. Pair each with its
    // FIRST (preferred) suggestion.
    let mut edits: Vec<(Span<char>, &Suggestion)> = lints
        .iter()
        .filter(|l| l.span.start <= l.span.end && l.span.end <= char_count)
        .filter(|l| !matches!(l.lint_kind, LintKind::Enhancement))
        .filter_map(|l| l.suggestions.first().map(|s| (l.span, s)))
        .collect();

    // Drop overlapping edits (greedy, earliest-wins) so applying one can't invalidate another.
    edits.sort_by_key(|(span, _)| span.start);
    let mut kept: Vec<(Span<char>, &Suggestion)> = Vec::new();
    let mut last_end = 0usize;
    for (span, sug) in edits {
        if span.start >= last_end {
            last_end = span.end;
            kept.push((span, sug));
        }
    }

    // Apply from the END backward so each edit's char-span stays valid despite length changes.
    let mut chars: Vec<char> = text.chars().collect();
    for (span, sug) in kept.into_iter().rev() {
        sug.apply(span, &mut chars);
    }
    chars.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_is_unchanged() {
        assert_eq!(harper_cleanup(""), "");
        assert_eq!(harper_cleanup("   "), "   ");
    }

    #[test]
    fn corrects_a_versus_an() {
        // Harper's canonical fix — proves it actually applies a real grammar correction.
        assert_eq!(harper_cleanup("This is an test."), "This is a test.");
    }

    #[test]
    fn fixes_a_common_misspelling() {
        // "recieve" -> "receive" (spelling correction with matched casing).
        assert_eq!(
            harper_cleanup("Please recieve this."),
            "Please receive this."
        );
    }

    #[test]
    fn leaves_correct_text_unchanged() {
        // A realness check: clean prose must survive untouched.
        let clean = "The patient is stable and doing well.";
        assert_eq!(harper_cleanup(clean), clean);
    }

    #[test]
    fn analyze_reports_the_an_issue_with_a_replacement() {
        let suggestions = analyze("This is an test.");
        let an = suggestions
            .iter()
            .find(|s| s.original.eq_ignore_ascii_case("an"))
            .expect("should flag the incorrect 'an'");
        assert!(an.start < an.end, "span must be non-empty");
        assert!(
            an.replacements.iter().any(|r| r == "a"),
            "should suggest 'a', got {:?}",
            an.replacements
        );
    }

    #[test]
    fn analyze_is_empty_on_blank_and_clean_text() {
        assert!(analyze("").is_empty());
        assert!(analyze("   ").is_empty());
        assert!(
            analyze("The patient is stable and doing well.").is_empty(),
            "clean prose should have no suggestions"
        );
    }
}
