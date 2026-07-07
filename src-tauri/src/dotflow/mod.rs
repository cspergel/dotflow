//! DotFlow — the deterministic text pipeline (design §7 "deterministic cleanup", §11c, §12).
//!
//! This is DotFlow's ORIGINAL wedge code layered on the Handy fork: the pure, no-LLM transform that turns
//! a raw dictated clause into the text to inject — phrase expansion (`.copd` / "insert copd plan") + the
//! spoken-punctuation command table. It sits between transcription and injection:
//!
//!   raw clause ─► [expand phrases] ─► [punctuation mode] ─► text to inject (enigo, per clause)
//!
//! Every function here is pure + deterministic + total (no IO/clock/global state), so it is fully
//! unit-testable AND gradable by the DTF referee. The effectful shell (SQLite phrase library, the live
//! injection cadence, the overlay) wraps this core; it is intentionally NOT in this module.

pub mod field_stream;
pub mod phrases;
pub mod punctuation;

pub use field_stream::FieldStreamer;
pub use phrases::{expand, Phrase, PhraseTable};
pub use punctuation::{apply_spoken, PunctuationMode};

/// Process one dictated clause into the text DotFlow will inject. Expansion runs FIRST (so a command
/// clause like "insert copd plan" resolves to its template as one clean block, §11c), THEN the
/// punctuation mode is applied to the result. Pure + total.
///
/// Ordering note: expanding before punctuation keeps voice-alias matching intact (spoken-punctuation
/// would otherwise attach a mark to the last alias word and break the match). Known V1 edge: a template
/// that itself contains a bare spoken-mark word (e.g. the literal word "period") would be converted in
/// Spoken mode — acceptable for V1; templates are authored text and rarely contain bare mark words.
pub fn process_clause(raw: &str, mode: PunctuationMode, table: &PhraseTable) -> String {
    let expanded = expand(raw, table);
    match mode {
        PunctuationMode::Spoken => apply_spoken(&expanded),
        PunctuationMode::Auto | PunctuationMode::Raw => expanded,
    }
}

/// DotFlow's built-in starter phrase pack (design §8 — the coder launch pack + a few general phrases).
/// A first-run default; the editable per-user library (SQLite) supersedes it later. Voice-aliases mirror
/// the dot triggers so a phrase can be spoken ("insert follow up") or typed (`.fu`).
pub fn starter_pack() -> PhraseTable {
    PhraseTable::new(&starter_pack_phrases())
}

/// The starter pack as a raw phrase list — used to SEED the editable library on first run (the user then
/// owns/edits these). `starter_pack()` builds the compiled table from the same list.
pub fn starter_pack_phrases() -> Vec<Phrase> {
    let p = |key: &str, aliases: &[&str], expansion: &str| Phrase {
        key: key.into(),
        aliases: aliases.iter().map(|s| s.to_string()).collect(),
        expansion: expansion.into(),
    };
    vec![
        // Coder pack (the launch audience): prompt prefixes for dictating into an AI IDE / chat box.
        p("fix", &["insert fix"], "Fix the bug where "),
        p("refactor", &["insert refactor"], "Refactor this so that "),
        p(
            "test",
            &["insert test", "write a test"],
            "Write a test for ",
        ),
        p(
            "explain",
            &["insert explain"],
            "Explain what this code does and why: ",
        ),
        p(
            "commit",
            &["insert commit"],
            "Write a concise commit message for these changes.",
        ),
        p(
            "pr",
            &["insert pull request", "insert pr"],
            "Summarize this change as a pull-request description.",
        ),
        p("todo", &["insert todo"], "TODO: "),
        // General.
        p(
            "ty",
            &["insert thanks"],
            "Thanks so much — really appreciate it.",
        ),
        p(
            "fu",
            &["insert follow up"],
            "Following up on this — let me know if you need anything else.",
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table() -> PhraseTable {
        PhraseTable::new(&[Phrase {
            key: "copd".into(),
            aliases: vec!["insert copd plan".into()],
            expansion: "COPD: continue inhalers, follow up pulmonary.".into(),
        }])
    }

    #[test]
    fn starter_pack_expands_a_coder_and_a_general_trigger() {
        let t = starter_pack();
        assert_eq!(expand(".todo", &t), "TODO: ");
        assert_eq!(expand("insert follow up", &t), expand(".fu", &t));
    }

    #[test]
    fn command_clause_resolves_to_the_template_block_in_any_mode() {
        let t = table();
        let exp = "COPD: continue inhalers, follow up pulmonary.";
        assert_eq!(
            process_clause("insert copd plan", PunctuationMode::Auto, &t),
            exp
        );
        assert_eq!(process_clause(".copd", PunctuationMode::Raw, &t), exp);
    }

    #[test]
    fn free_dictation_with_spoken_punctuation() {
        let t = table();
        assert_eq!(
            process_clause("the patient is stable period", PunctuationMode::Spoken, &t),
            "the patient is stable."
        );
    }

    #[test]
    fn raw_mode_leaves_spoken_words_untouched_for_coders() {
        // A coder in Raw mode dictating into an IDE keeps "period" as a literal word (no mangling).
        let t = table();
        assert_eq!(
            process_clause("call foo period bar", PunctuationMode::Raw, &t),
            "call foo period bar"
        );
    }

    #[test]
    fn auto_mode_trusts_the_models_native_punctuation() {
        let t = table();
        assert_eq!(
            process_clause("The patient is stable.", PunctuationMode::Auto, &t),
            "The patient is stable."
        );
    }
}
