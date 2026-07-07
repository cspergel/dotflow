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
}
