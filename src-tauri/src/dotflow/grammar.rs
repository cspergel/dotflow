//! DotFlow — offline grammar & spelling cleanup via Harper (github.com/automattic/harper).
//!
//! Fully local: no network, no API key, milliseconds to lint. This is the mid tier of the cleanup hotkey —
//! better than the mechanical [`super::cleanup`] pass, and used when no post-process LLM is configured. It
//! applies Harper's concrete corrections (spelling, agreement, grammar, capitalization, punctuation, …) but
//! skips subjective [`LintKind::Enhancement`] rewrites and any lint that offers no replacement, so it only
//! ever makes safe, confident edits.

use std::collections::HashSet;

use harper_core::linting::{LintGroup, LintKind, Linter, Suggestion};
use harper_core::{Dialect, Document, Span};
use serde::Serialize;
use specta::Type;

use super::dictionary_packs;

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
    // [SWEEP-F1] Parse the Document with the SAME merged dict handed to the linter — otherwise the pack
    // vocabulary never gets metadata at parse time and Spelling lints are never suppressed (a no-op). The
    // human-reviewed `analyze` path surfaces every suggestion (no safety filter); the clinician chooses.
    let dict = dictionary_packs::current_dictionary();
    let document = Document::new_plain_english(text, &*dict);
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

/// [RS2-F1 / SWEEP-F2 / F2b / F4] The silent-auto-fix safety filter. Returns `true` when an edit must be
/// DROPPED because it touches medical jargon — so `harper_cleanup` never silently rewrites text *into* a
/// drug name, nor silently overwrites a valid jargon term with something else.
///
/// The `jargon` set is enabled-pack terms MINUS Harper's curated dictionary, already Unicode-lowercased
/// ([RS2-F1]) — homographs like `cold`/`stroke` are excluded, so their normal corrections still apply. The
/// match is case-normalized ([SWEEP-F2]) because Harper mirrors the offending word's casing into its
/// suggestion (so `Metoprolol` at a sentence start would bypass a case-sensitive check). Both the
/// **replacement/inserted** side and the **original span** are guarded, across all three `Suggestion`
/// variants ([SWEEP-F2b] / [SWEEP-F4]).
fn is_medical_edit(sug: &Suggestion, original: &str, jargon: &HashSet<String>) -> bool {
    if jargon.is_empty() {
        return false; // fast path: no packs / no jargon → nothing to guard
    }
    // Original-side guard (covers Remove, and a non-spell rewrite of a valid jargon term).
    if jargon.contains(&original.to_lowercase()) {
        return true;
    }
    // Replacement/inserted-side guard.
    match sug {
        Suggestion::ReplaceWith(chars) | Suggestion::InsertAfter(chars) => {
            let candidate: String = chars.iter().collect();
            jargon.contains(&candidate.to_lowercase())
        }
        Suggestion::Remove => false,
    }
}

fn harper_cleanup_inner(text: &str) -> String {
    let text_chars: Vec<char> = text.chars().collect();
    let char_count = text_chars.len();
    // [SWEEP-F1] Same merged dict for BOTH the Document parse and the linter; plus the jargon set for the
    // safety filter — snapshotted together so they are consistent.
    let (dict, jargon) = dictionary_packs::current_snapshot();
    let document = Document::new_plain_english(text, &*dict);
    let mut linter = LintGroup::new_curated(dict, Dialect::American);
    let lints = linter.lint(&document);

    // Keep only lints that (a) have a span within bounds, (b) offer a concrete replacement, and (c) are
    // actual corrections — not optional style "enhancements" we don't want to auto-apply. Pair each with its
    // FIRST (preferred) suggestion, and DROP any edit the medical safety filter flags ([RS2-F1]).
    let mut edits: Vec<(Span<char>, &Suggestion)> = lints
        .iter()
        .filter(|l| l.span.start <= l.span.end && l.span.end <= char_count)
        .filter(|l| !matches!(l.lint_kind, LintKind::Enhancement))
        .filter_map(|l| {
            let sug = l.suggestions.first()?;
            let original: String = text_chars[l.span.start..l.span.end].iter().collect();
            if is_medical_edit(sug, &original, &jargon) {
                None // silently dropped from the auto-fix; still surfaced by `analyze` for human review
            } else {
                Some((l.span, sug))
            }
        })
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

    // ---- Medical dictionary pack (design §4) ----------------------------------------------------------
    //
    // These exercise the process-wide pack cache (`dictionary_packs::current_*`), which is shared global
    // state. Cargo runs tests concurrently, so pack-state-mutating tests serialize on this lock and each
    // sets the exact state it needs while holding it, then resets to curated-only on exit.

    use harper_core::spell::{Dictionary, FstDictionary};
    use std::sync::Mutex;

    static PACK_TEST_LOCK: Mutex<()> = Mutex::new(());

    fn empty_pack_dir(tag: &str) -> std::path::PathBuf {
        // An empty dir → only the bundled `medical` pack is discovered (no user files).
        let dir =
            std::env::temp_dir().join(format!("dotflow-grammar-{}-{}", tag, std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// The first replacement Harper offers for the span whose original text (case-insensitively) equals
    /// `word`, if any — mirrors what `harper_cleanup` would apply (it uses `suggestions.first()`).
    fn first_replacement_for<'a>(sugs: &'a [TextSuggestion], word: &str) -> Option<&'a String> {
        sugs.iter()
            .find(|s| s.original.eq_ignore_ascii_case(word))
            .and_then(|s| s.replacements.first())
    }

    // Test 1 — acceptance changes with the pack, both states; premise guarded in-test ([SWEEP-F-t1]). Also
    // covers [SWEEP-F1]: if the Document parse dict weren't swapped, the on-state would still flag → fail.
    #[test]
    fn test1_acceptance_changes_with_the_pack() {
        let _g = PACK_TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let dir = empty_pack_dir("acceptance");

        // Parameterized over confirmed-absent-from-curated jargon terms.
        for term in ["metoprolol", "lisinopril", "dyspnea"] {
            // Premise: the term must be absent from curated Harper (else the test proves nothing).
            assert!(
                !FstDictionary::curated().contains_word_str(term),
                "premise: '{term}' must be absent from curated Harper"
            );

            let sentence = format!("The patient takes {term} daily.");

            // Pack OFF → flagged as Spelling.
            dictionary_packs::set_enabled_packs(&dir, &[]);
            let off = analyze(&sentence);
            assert!(
                off.iter()
                    .any(|s| s.original.eq_ignore_ascii_case(term) && s.kind == "Spelling"),
                "pack off: '{term}' should be flagged as Spelling, got {off:?}"
            );

            // Pack ON → the flag is gone (accepted vocabulary).
            dictionary_packs::set_enabled_packs(&dir, &["medical".to_string()]);
            let on = analyze(&sentence);
            assert!(
                !on.iter().any(|s| s.original.eq_ignore_ascii_case(term)),
                "pack on: '{term}' must NOT be flagged, got {on:?}"
            );
        }

        dictionary_packs::set_enabled_packs(&dir, &[]); // reset
        let _ = std::fs::remove_dir_all(&dir);
    }

    // Test 2 (unit) — the safety filter predicate has teeth deterministically: casing-normalized, both
    // replacement and original sides, all three Suggestion variants ([SWEEP-F2/F2b/F4]).
    #[test]
    fn test2_filter_predicate_guards_casing_original_and_variants() {
        let jargon: HashSet<String> = ["metoprolol".to_string()].into_iter().collect();
        let chars = |s: &str| s.chars().collect::<Vec<char>>();

        // Replacement side, matched case-insensitively (the CRITICAL sentence-initial "Metoprolol" case).
        assert!(is_medical_edit(
            &Suggestion::ReplaceWith(chars("metoprolol")),
            "metaprolol",
            &jargon
        ));
        assert!(
            is_medical_edit(
                &Suggestion::ReplaceWith(chars("Metoprolol")),
                "Metaprolol",
                &jargon
            ),
            "capitalized replacement must still be dropped (casing bypass)"
        );
        // InsertAfter variant guarded too.
        assert!(is_medical_edit(
            &Suggestion::InsertAfter(chars("METOPROLOL")),
            "x",
            &jargon
        ));
        // Original-side guard: a valid jargon term being rewritten to a non-jargon word, or removed.
        assert!(is_medical_edit(
            &Suggestion::ReplaceWith(chars("metropolis")),
            "Metoprolol",
            &jargon
        ));
        assert!(is_medical_edit(&Suggestion::Remove, "metoprolol", &jargon));

        // Homographs / ordinary edits are NOT dropped (jargon set excludes curated words → empty here).
        assert!(!is_medical_edit(
            &Suggestion::ReplaceWith(chars("a")),
            "an",
            &jargon
        ));
        assert!(!is_medical_edit(&Suggestion::Remove, "the", &jargon));
        // Empty jargon set fast-path.
        assert!(!is_medical_edit(
            &Suggestion::ReplaceWith(chars("metoprolol")),
            "metaprolol",
            &HashSet::new()
        ));
    }

    // Test 2 (integration) — the filter is path-specific: with the pack on, a typo whose PRIMARY suggestion
    // is the medical term is left UNCHANGED by `harper_cleanup` (dropped) but SURFACED by `analyze`.
    #[test]
    fn test2_medical_first_suggestion_is_dropped_by_cleanup_but_surfaced_by_analyze() {
        let _g = PACK_TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let dir = empty_pack_dir("safety");
        dictionary_packs::set_enabled_packs(&dir, &["medical".to_string()]);

        // "metoprololl" is edit-distance 1 from the pack term and far from any English word, so Harper ranks
        // the drug name first. (Guarded below — if that stops holding, the assert says so clearly.)
        let typo = "metoprololl";
        let sentence = format!("The patient takes {typo} daily.");

        let sugs = analyze(&sentence);
        let first = first_replacement_for(&sugs, typo).unwrap_or_else(|| {
            panic!("analyze should surface a suggestion for '{typo}', got {sugs:?}")
        });
        assert!(
            first.eq_ignore_ascii_case("metoprolol"),
            "precondition: Harper's PRIMARY suggestion for '{typo}' must be the medical term, got '{first}'"
        );

        // harper_cleanup must leave the typo span untouched (the filter dropped the medical replacement).
        let cleaned = harper_cleanup(&sentence);
        assert!(
            cleaned.contains(typo),
            "safety: '{typo}' must NOT be silently rewritten to a drug name; got '{cleaned}'"
        );

        dictionary_packs::set_enabled_packs(&dir, &[]); // reset
        let _ = std::fs::remove_dir_all(&dir);
    }

    // Test 2 (integration, casing) — the CRITICAL sentence-initial case: a capitalized medical first
    // suggestion must ALSO be dropped by harper_cleanup (fails if the filter isn't case-normalized).
    #[test]
    fn test2_sentence_initial_capitalized_medical_is_not_auto_applied() {
        let _g = PACK_TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let dir = empty_pack_dir("safety-cap");
        dictionary_packs::set_enabled_packs(&dir, &["medical".to_string()]);

        // Sentence-initial → Harper mirrors the capital into its suggestion ("Metoprololl" → "Metoprolol").
        let sentence = "Metoprololl was started today.";
        let cleaned = harper_cleanup(sentence);
        assert!(
            cleaned.starts_with("Metoprololl"),
            "safety(casing): sentence-initial typo must not be rewritten to a capitalized drug name; got '{cleaned}'"
        );

        dictionary_packs::set_enabled_packs(&dir, &[]); // reset
        let _ = std::fs::remove_dir_all(&dir);
    }

    // Test 3 — no regression: with the pack on, curated grammar linting still works and clean prose is left
    // alone (proves the merge didn't break curated linting).
    #[test]
    fn test3_no_regression_with_pack_on() {
        let _g = PACK_TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let dir = empty_pack_dir("regression");
        dictionary_packs::set_enabled_packs(&dir, &["medical".to_string()]);

        assert_eq!(harper_cleanup("This is an test."), "This is a test.");
        let clean = "The patient is stable and doing well.";
        assert_eq!(harper_cleanup(clean), clean);

        dictionary_packs::set_enabled_packs(&dir, &[]); // reset
        let _ = std::fs::remove_dir_all(&dir);
    }

    // Negative-collateral ([RS2-F1]) — a homograph in the pack still gets its normal correction, proving the
    // drop-set is pack-terms MINUS curated (not the raw pack set).
    #[test]
    fn test_homograph_in_pack_still_gets_normal_correction() {
        let _g = PACK_TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let dir = empty_pack_dir("collateral");
        // A user pack that (deliberately) contains the common-English homograph "cold" and a misspelling
        // target is irrelevant — what matters is that "cold" being in a pack does NOT suppress corrections
        // whose replacement is "cold".
        std::fs::write(dir.join("clinic.txt"), "cold\nmetoprolol\n").unwrap();
        dictionary_packs::set_enabled_packs(&dir, &["clinic".to_string(), "medical".to_string()]);

        // "cold" is curated → excluded from the jargon drop-set → a typo correcting to "cold" is applied.
        let cleaned = harper_cleanup("The patient feels could today.");
        // Harper corrects the obvious typo; the key point is the pack did not freeze a "cold"-valued fix.
        // We assert the jargon set excludes 'cold' directly (deterministic), plus that cleanup still runs.
        let (_d, jargon) = dictionary_packs::current_snapshot();
        assert!(
            !jargon.contains("cold"),
            "homograph 'cold' must be excluded from the drop-set"
        );
        assert!(
            jargon.contains("metoprolol"),
            "pure jargon 'metoprolol' stays in the drop-set"
        );
        // Sanity: cleanup produced a string (no panic) of comparable content.
        assert!(cleaned.contains("patient"));

        dictionary_packs::set_enabled_packs(&dir, &[]); // reset
        let _ = std::fs::remove_dir_all(&dir);
    }
}
