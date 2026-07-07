//! DotFlow — stream a streaming model's COMMITTED text into the focused text field, EXPANDED (design §11b +
//! §11c: the Dragon feel AND "parse-and-expand before inject; command-buffer for triggers spanning a pause").
//!
//! Handy's streaming worker separates **committed** text (finalized) from **tentative** text (the changing
//! guess). We mirror the committed text into the field with three properties:
//!
//!   1. **Append-only.** We only inject the growing tail; a revision of already-injected text is skipped
//!      (the overlay shows the corrected guess; the final text is synced at stop).
//!   2. **Whole-words-only + throttled by the caller.** We hold the trailing PARTIAL word (no trailing
//!      space yet) and release only finished words, so keystroke bursts stay large and few — the caller also
//!      throttles them ≥100ms apart. Together this is what keeps the OS input queue from racing (the cause
//!      of dropped/repeated keys like "ppppp"). The held last word flushes at stop.
//!   3. **Parse-and-expand before inject (the command-buffer).** A phrase lands as one clean block, never
//!      typed word-by-word then stranded. We also hold a trailing run of finished words while it is still a
//!      PROPER PREFIX of some voice-alias ("insert follow" → wait for "up"); the moment the trigger
//!      completes it is expanded and injected as a unit, and if the run turns out not to be a trigger the
//!      words release raw. A trigger that ends the utterance expands cleanly at stop.
//!
//! `FieldStreamer` is pure (a deterministic function of its inputs + its own tracked state) and unit-tested;
//! the enigo injection of the returned text is the glue.

use super::phrases::{expand, PhraseTable};

/// Tracks the committed text already CONSUMED (turned into field text) this dictation, plus the last char
/// injected (for correct spacing between the chunks we release). One per dictation; `reset` at stream start,
/// `flush` at stop.
pub struct FieldStreamer {
    /// The raw committed prefix we have already released into the field (bookkeeping for the append-only
    /// guard and for locating the un-injected tail). NOT what's literally in the field — that is the
    /// expansion of this — but it always stays a prefix of `committed`.
    consumed: String,
    /// The last character actually injected into the field, so a released chunk gets exactly one separating
    /// space (and none after a newline or an existing trailing space).
    last_char: Option<char>,
}

impl Default for FieldStreamer {
    fn default() -> Self {
        FieldStreamer { consumed: String::new(), last_char: None }
    }
}

impl FieldStreamer {
    pub fn new() -> Self {
        Self::default()
    }

    /// Start a fresh dictation.
    pub fn reset(&mut self) {
        self.consumed.clear();
        self.last_char = None;
    }

    /// Word-batched, phrase-aware, append-only sync. Given the model's FULL committed text so far, return
    /// `(0, text_to_inject)` — the expansion of the newly-releasable words. Holds back (a) a trailing
    /// partial word and (b) a trailing run of complete words that is still a proper prefix of a voice-alias
    /// (the command-buffer), so a phrase lands as one block. `backspaces` is always 0.
    pub fn advance(&mut self, committed: &str, table: &PhraseTable) -> (usize, String) {
        if !committed.starts_with(&self.consumed) {
            // A revision of already-injected text — skip (no churn); keep `consumed` so a later pure
            // extension still appends cleanly.
            return (0, String::new());
        }
        let pending = &committed[self.consumed.len()..];
        let spans = token_spans(pending);
        if spans.is_empty() {
            return (0, String::new()); // nothing but whitespace yet — release with the next word
        }
        let ends_ws = pending.ends_with(char::is_whitespace);
        let n = spans.len();
        let complete = if ends_ws { n } else { n - 1 }; // trailing partial word (no space yet) is held
        let toks: Vec<&str> = spans.iter().map(|&(a, b)| &pending[a..b]).collect();

        // Command-buffer: also hold the longest trailing run of FINISHED words that is still a proper alias
        // prefix ("insert follow" → wait for "up"), so a phrase lands as one block.
        let mut hold = 0usize;
        for h in (1..=complete).rev() {
            if table.is_partial_alias(&toks[complete - h..complete]) {
                hold = h;
                break;
            }
        }
        let held_start_tok = complete - hold; // first finished word that is held

        let cut = if held_start_tok < complete {
            spans[held_start_tok].0 // release up to the start of the held alias-prefix run
        } else if !ends_ws {
            spans[complete].0 // nothing buffered, but hold the trailing partial word
        } else {
            pending.len() // release every finished word (incl. trailing space)
        };
        if cut == 0 {
            return (0, String::new()); // everything is still held
        }
        let releasable = &pending[..cut];
        let to_type = self.render(releasable, table);
        self.consumed.push_str(releasable);
        (0, to_type)
    }

    /// Release everything remaining at STOP — the held trigger/partial word included — expanded. Skips on a
    /// non-extension revision.
    pub fn flush(&mut self, committed: &str, table: &PhraseTable) -> (usize, String) {
        if !committed.starts_with(&self.consumed) {
            return (0, String::new());
        }
        let remaining = committed[self.consumed.len()..].to_string();
        let to_type = self.render(&remaining, table);
        self.consumed = committed.to_string();
        (0, to_type)
    }

    /// Expand a released raw chunk and prefix exactly one separating space when the field's last char needs
    /// it (not at the very start, and not after a newline or an existing trailing space). Updates `last_char`.
    fn render(&mut self, raw: &str, table: &PhraseTable) -> String {
        let e = expand(raw, table);
        if e.is_empty() {
            return String::new();
        }
        let mut out = String::new();
        if let Some(c) = self.last_char {
            if c != '\n' && c != ' ' {
                out.push(' ');
            }
        }
        out.push_str(&e);
        self.last_char = out.chars().last();
        out
    }
}

/// Byte spans of whitespace-delimited tokens in `s`.
fn token_spans(s: &str) -> Vec<(usize, usize)> {
    let mut spans = Vec::new();
    let mut start: Option<usize> = None;
    for (i, c) in s.char_indices() {
        if c.is_whitespace() {
            if let Some(st) = start.take() {
                spans.push((st, i));
            }
        } else if start.is_none() {
            start = Some(i);
        }
    }
    if let Some(st) = start {
        spans.push((st, s.len()));
    }
    spans
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dotflow::phrases::Phrase;

    fn empty() -> PhraseTable {
        PhraseTable::default()
    }

    fn table() -> PhraseTable {
        PhraseTable::new(&[
            Phrase {
                key: "copd".into(),
                aliases: vec!["insert copd plan".into()],
                expansion: "COPD: continue inhalers, follow up pulmonary.".into(),
            },
            Phrase {
                key: "fu".into(),
                aliases: vec!["insert follow up".into()],
                expansion: "Follow up in two weeks.".into(),
            },
        ])
    }

    // ── free dictation (no phrases): whole words only; the trailing partial word is held until finished ──
    #[test]
    fn holds_the_trailing_partial_word_until_a_boundary_completes_it() {
        let mut s = FieldStreamer::new();
        let t = empty();
        assert_eq!(s.advance("the", &t), (0, String::new())); // "the" still in progress → held
        assert_eq!(s.advance("the patient", &t), (0, "the".to_string())); // "the" finished; "patient" held
        assert_eq!(s.advance("the patient is", &t), (0, " patient".to_string()));
        assert_eq!(s.flush("the patient is stable", &t), (0, " is stable".to_string()));
    }

    #[test]
    fn finished_words_release_together() {
        let mut s = FieldStreamer::new();
        let t = empty();
        assert_eq!(s.advance("the patient is stable and ", &t), (0, "the patient is stable and".to_string()));
        assert_eq!(s.advance("the patient is stable and improving well ", &t), (0, " improving well".to_string()));
    }

    #[test]
    fn a_revision_of_already_injected_text_is_skipped() {
        let mut s = FieldStreamer::new();
        let t = empty();
        assert_eq!(s.advance("the pay ", &t), (0, "the pay".to_string()));
        assert_eq!(s.advance("the patient ", &t), (0, String::new())); // not an extension → skip
        assert_eq!(s.advance("the pay later ", &t), (0, " later".to_string()));
    }

    #[test]
    fn no_new_finished_word_is_a_no_op() {
        let mut s = FieldStreamer::new();
        let t = empty();
        s.advance("hello world ", &t);
        assert_eq!(s.advance("hello world ", &t), (0, String::new()));
        assert_eq!(s.advance("hello world how", &t), (0, String::new())); // "how" still in progress
    }

    // ── dot-triggers (single token: held as a partial word until finished, then expands as a block) ──────
    #[test]
    fn dot_trigger_expands_as_one_block_once_the_word_finishes() {
        let mut s = FieldStreamer::new();
        let t = table();
        // ".copd" has no trailing space yet → held; the preceding finished word releases.
        assert_eq!(s.advance("assessment .copd", &t), (0, "assessment".to_string()));
        // once a following word arrives, ".copd" is finished → expands as a block; "done" is now held.
        assert_eq!(s.advance("assessment .copd done", &t),
            (0, " COPD: continue inhalers, follow up pulmonary.".to_string()));
    }

    // ── voice-aliases (multi-word → the command-buffer) ─────────────────────────────────────────────────
    #[test]
    fn a_spoken_alias_is_held_word_by_word_then_lands_as_one_block() {
        let mut s = FieldStreamer::new();
        let t = table();
        // "insert" is a proper prefix of aliases → held (nothing injected).
        assert_eq!(s.advance("insert ", &t), (0, String::new()));
        // "insert copd" still a proper prefix of "insert copd plan" → held.
        assert_eq!(s.advance("insert copd ", &t), (0, String::new()));
        // "insert copd plan" completes → expands as one clean block.
        assert_eq!(s.advance("insert copd plan ", &t),
            (0, "COPD: continue inhalers, follow up pulmonary.".to_string()));
    }

    #[test]
    fn an_alias_prefix_that_turns_out_not_to_be_a_trigger_is_released_raw() {
        let mut s = FieldStreamer::new();
        let t = table();
        assert_eq!(s.advance("insert ", &t), (0, String::new())); // held (prefix)
        // "insert copies" is not a prefix of any alias → release the words raw.
        assert_eq!(s.advance("insert copies ", &t), (0, "insert copies".to_string()));
    }

    #[test]
    fn free_words_before_a_trigger_are_released_the_trigger_still_buffers() {
        let mut s = FieldStreamer::new();
        let t = table();
        // "the plan is" releases; "insert" starts buffering.
        assert_eq!(s.advance("the plan is insert ", &t), (0, "the plan is".to_string()));
        assert_eq!(s.advance("the plan is insert follow ", &t), (0, String::new())); // "insert follow" buffers
        assert_eq!(s.advance("the plan is insert follow up ", &t),
            (0, " Follow up in two weeks.".to_string()));
    }

    #[test]
    fn flush_releases_a_dangling_alias_prefix_raw() {
        let mut s = FieldStreamer::new();
        let t = table();
        assert_eq!(s.advance("insert copd ", &t), (0, String::new())); // buffered
        // user stops mid-trigger → the buffered words are typed raw (not a complete alias).
        assert_eq!(s.flush("insert copd", &t), (0, "insert copd".to_string()));
    }

    // ── misc ────────────────────────────────────────────────────────────────────────────────────────────
    #[test]
    fn reset_starts_a_fresh_dictation() {
        let mut s = FieldStreamer::new();
        let t = empty();
        s.advance("first dictation done ", &t);
        s.reset();
        assert_eq!(s.advance("second one ", &t), (0, "second one".to_string()));
    }

    #[test]
    fn is_total_on_empty_and_utf8_safe() {
        let mut s = FieldStreamer::new();
        let t = empty();
        assert_eq!(s.advance("", &t), (0, String::new()));
        assert_eq!(s.advance("café ", &t), (0, "café".to_string()));
        assert_eq!(s.flush("café au lait", &t), (0, " au lait".to_string()));
    }
}
