//! DotFlow — the TYPED text expander (Beeftext / Espanso‑style): type a dot‑trigger (`.fu`) in ANY app and
//! it is replaced by your saved text, using the SAME phrase library that powers spoken triggers.
//!
//! This module holds the PURE, unit‑tested core: a rolling `ExpanderBuffer` of recently typed characters and
//! the trigger match against `PhraseTable`. The OS glue — a global keyboard monitor (Windows Raw Input) that
//! feeds characters in, and the backspace‑then‑paste emit — lives in a separate, opt‑in backend and is
//! **off by default** (`experimental_typed_expander`). Nothing here runs unless the user enables it, so it
//! has zero effect on the dictation path.
//!
//! Self‑trigger safety: while DotFlow injects text (dictation streaming, the finalize paste, or this
//! expander's own emit) `crate::clipboard::is_injecting()` is raised and the monitor drops all input, so
//! DotFlow's own keystrokes/paste can never re‑trigger an expansion.

use super::phrases::PhraseTable;

/// Cap on retained characters — comfortably longer than any trigger, so the buffer stays bounded.
const MAX_BUFFER_CHARS: usize = 64;

/// A rolling buffer of recently typed characters. The caller (the keyboard backend) pushes printable chars,
/// applies `backspace` on Backspace, and `reset` on a "combo breaker" (Enter/Tab/Esc, arrow/navigation keys,
/// a mouse click, or a window‑focus change). Pure + total.
#[derive(Default)]
pub struct ExpanderBuffer {
    buf: String,
}

impl ExpanderBuffer {
    pub fn new() -> Self {
        Self::default()
    }

    /// Clear the buffer (a combo breaker occurred).
    pub fn reset(&mut self) {
        self.buf.clear();
    }

    /// The user pressed Backspace — drop the last character (mid‑word editing stays correct).
    pub fn backspace(&mut self) {
        self.buf.pop();
    }

    /// Append a typed character, keeping only the last `MAX_BUFFER_CHARS`.
    pub fn push(&mut self, c: char) {
        self.buf.push(c);
        let len = self.buf.chars().count();
        if len > MAX_BUFFER_CHARS {
            self.buf = self.buf.chars().skip(len - MAX_BUFFER_CHARS).collect();
        }
    }

    /// If the buffer now ends with a known dot‑trigger, return `(chars_to_delete, expansion)` — how many
    /// characters the backend must backspace (the `.key`) and the replacement to paste.
    pub fn matched(&self, table: &PhraseTable) -> Option<(usize, String)> {
        table.match_typed_trigger(&self.buf)
    }

    /// After the backend has replaced a matched trigger, drop those `chars` trailing characters from the
    /// buffer so the (already‑pasted) expansion text isn't re‑examined. The pasted text arrives via clipboard
    /// (not keystrokes) so it never enters the buffer; this just removes the consumed `.key`.
    pub fn consume(&mut self, chars: usize) {
        let keep = self.buf.chars().count().saturating_sub(chars);
        self.buf = self.buf.chars().take(keep).collect();
    }

    #[cfg(test)]
    fn contents(&self) -> &str {
        &self.buf
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dotflow::phrases::Phrase;

    fn table() -> PhraseTable {
        PhraseTable::new(&[
            Phrase {
                key: "fu".into(),
                aliases: vec![],
                expansion: "Follow up in two weeks.".into(),
            },
            Phrase {
                key: "copd".into(),
                aliases: vec![],
                expansion: "COPD plan.".into(),
            },
        ])
    }

    fn type_str(b: &mut ExpanderBuffer, s: &str) {
        for c in s.chars() {
            b.push(c);
        }
    }

    #[test]
    fn matches_a_dot_trigger_at_the_end() {
        let mut b = ExpanderBuffer::new();
        let t = table();
        type_str(&mut b, "the plan .fu");
        // `.fu` is 3 characters to delete; the expansion is the follow-up text.
        assert_eq!(
            b.matched(&t),
            Some((3, "Follow up in two weeks.".to_string()))
        );
    }

    #[test]
    fn no_match_until_the_full_trigger_is_typed() {
        let mut b = ExpanderBuffer::new();
        let t = table();
        type_str(&mut b, ".f");
        assert_eq!(b.matched(&t), None);
        b.push('u');
        assert!(b.matched(&t).is_some());
    }

    #[test]
    fn case_insensitive_and_longest_key_wins() {
        let mut b = ExpanderBuffer::new();
        let t = table();
        type_str(&mut b, "note .COPD");
        assert_eq!(b.matched(&t), Some((5, "COPD plan.".to_string()))); // ".copd" = 5 chars
    }

    #[test]
    fn backspace_edits_the_buffer_and_can_undo_a_match() {
        let mut b = ExpanderBuffer::new();
        let t = table();
        type_str(&mut b, ".fu");
        assert!(b.matched(&t).is_some());
        b.backspace(); // now ".f"
        assert_eq!(b.matched(&t), None);
    }

    #[test]
    fn reset_clears_the_buffer() {
        let mut b = ExpanderBuffer::new();
        let t = table();
        type_str(&mut b, ".fu");
        b.reset();
        assert_eq!(b.matched(&t), None);
        assert_eq!(b.contents(), "");
    }

    #[test]
    fn consume_drops_the_matched_trigger() {
        let mut b = ExpanderBuffer::new();
        type_str(&mut b, "hi .fu");
        b.consume(3); // remove ".fu"
        assert_eq!(b.contents(), "hi ");
    }

    #[test]
    fn an_unknown_dot_word_never_matches() {
        let mut b = ExpanderBuffer::new();
        let t = table();
        type_str(&mut b, ".unknown");
        assert_eq!(b.matched(&t), None);
    }

    #[test]
    fn buffer_stays_bounded() {
        let mut b = ExpanderBuffer::new();
        let t = table();
        type_str(&mut b, &"x".repeat(200));
        type_str(&mut b, ".fu");
        assert!(b.contents().chars().count() <= MAX_BUFFER_CHARS);
        assert!(b.matched(&t).is_some()); // trailing trigger still matches
    }
}
