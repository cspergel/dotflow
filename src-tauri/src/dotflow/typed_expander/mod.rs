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

#[cfg(target_os = "windows")]
pub mod backend;
#[cfg(target_os = "windows")]
pub use backend::ExpanderController;

/// Non-Windows stub: the typed-expander backend is a Windows Raw Input monitor, so on other platforms the
/// controller is a no-op (the setting still persists; enabling it just logs that the backend isn't built
/// yet). mac/Linux backends can slot in behind this same `start`/`stop` surface later.
#[cfg(not(target_os = "windows"))]
mod stub {
    use tauri::AppHandle;

    #[derive(Default)]
    pub struct ExpanderController;

    impl ExpanderController {
        pub fn new() -> Self {
            Self
        }
        pub fn start(&self, _app: AppHandle) {
            log::warn!(
                "Typed text expander is only implemented on Windows for now — ignoring enable."
            );
        }
        pub fn stop(&self) {}
    }
}
#[cfg(not(target_os = "windows"))]
pub use stub::ExpanderController;

/// Cap on retained characters — comfortably longer than any trigger, so the buffer stays bounded.
const MAX_BUFFER_CHARS: usize = 64;

/// What a keyboard event means to the expander buffer. The OS backend maps each raw key-DOWN to one of
/// these; this classification is pure (a `u16` virtual-key code in, a decision out) so it is unit-testable
/// without any OS. Windows `VK_*` codes are plain integers, so the constants below are portable literals.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyAction {
    /// Backspace — drop the last buffered char (`ExpanderBuffer::backspace`).
    Backspace,
    /// A "combo breaker" (Enter/Tab/Esc, an arrow/navigation key, Delete) — clear the buffer
    /// (`ExpanderBuffer::reset`). Continuity is broken, so a trigger can't span it.
    Reset,
    /// A key that MIGHT produce a printable character — the backend decodes it (`ToUnicodeEx`) and, if it
    /// yields a printable char, calls `ExpanderBuffer::push`.
    Decode,
    /// A key that never affects the buffer (a modifier held alone, Caps/Num/Scroll lock, a function key) —
    /// ignore it WITHOUT resetting, so holding Shift to type a capital doesn't wipe an in-progress trigger.
    Ignore,
}

// Windows virtual-key codes we special-case (values from WinUser.h; portable u16 literals).
const VK_BACK: u16 = 0x08;
const VK_TAB: u16 = 0x09;
const VK_RETURN: u16 = 0x0D;
const VK_ESCAPE: u16 = 0x1B;
const VK_PRIOR: u16 = 0x21; // Page Up
const VK_NEXT: u16 = 0x22; // Page Down
const VK_END: u16 = 0x23;
const VK_HOME: u16 = 0x24;
const VK_LEFT: u16 = 0x25;
const VK_UP: u16 = 0x26;
const VK_RIGHT: u16 = 0x27;
const VK_DOWN: u16 = 0x28;
const VK_INSERT: u16 = 0x2D;
const VK_DELETE: u16 = 0x2E;

/// Classify a virtual-key code (from a key-DOWN event) into its effect on the expander buffer. Pure + total.
///
/// - Backspace edits the buffer; Enter/Tab/Esc, the arrow/navigation cluster, and Delete break continuity
///   (reset). Modifiers, the lock keys, and function keys are ignored so they never disturb an in-progress
///   trigger. Everything else is a `Decode` candidate — the backend asks the OS whether it makes a char.
pub fn key_action(vk: u16) -> KeyAction {
    match vk {
        VK_BACK => KeyAction::Backspace,
        VK_RETURN | VK_TAB | VK_ESCAPE => KeyAction::Reset,
        VK_LEFT | VK_UP | VK_RIGHT | VK_DOWN | VK_PRIOR | VK_NEXT | VK_END | VK_HOME
        | VK_INSERT | VK_DELETE => KeyAction::Reset,
        // Modifiers (Shift/Ctrl/Alt and their L/R variants 0xA0–0xA5, plus the Windows keys 0x5B/0x5C) and
        // the lock keys — held alone they must NOT touch the buffer. When a modifier is combined with a
        // letter the OS still delivers the LETTER's own key event, which we Decode.
        0x10..=0x12 | 0x5B | 0x5C | 0xA0..=0xA5 => KeyAction::Ignore, // Shift/Ctrl/Alt, LWin/RWin
        0x14 | 0x90 | 0x91 => KeyAction::Ignore,                      // Caps/Num/Scroll lock
        0x70..=0x87 => KeyAction::Ignore,                             // F1–F24
        _ => KeyAction::Decode,
    }
}

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

    #[test]
    fn backspace_key_is_a_buffer_edit() {
        assert_eq!(key_action(VK_BACK), KeyAction::Backspace);
    }

    #[test]
    fn enter_tab_and_escape_reset_the_buffer() {
        assert_eq!(key_action(VK_RETURN), KeyAction::Reset);
        assert_eq!(key_action(VK_TAB), KeyAction::Reset);
        assert_eq!(key_action(VK_ESCAPE), KeyAction::Reset);
    }

    #[test]
    fn arrow_and_navigation_keys_reset_the_buffer() {
        // Moving the caret breaks trigger continuity — a partial `.f` before an arrow must not later
        // complete into `.fu` across the jump.
        for vk in [
            VK_LEFT, VK_UP, VK_RIGHT, VK_DOWN, VK_HOME, VK_END, VK_PRIOR, VK_NEXT, VK_INSERT,
            VK_DELETE,
        ] {
            assert_eq!(
                key_action(vk),
                KeyAction::Reset,
                "vk {vk:#04x} should reset"
            );
        }
    }

    #[test]
    fn modifiers_and_lock_keys_are_ignored_not_reset() {
        // Holding Shift to type a capital inside a trigger must NOT wipe the buffer.
        for vk in [
            0x10u16, // Shift
            0x11,    // Ctrl
            0x12,    // Alt
            0xA0,    // LShift
            0xA5,    // RAlt (AltGr)
            0x5B,    // LWin
            0x14,    // Caps Lock
            0x90,    // Num Lock
            0x70,    // F1
            0x87,    // F24
        ] {
            assert_eq!(
                key_action(vk),
                KeyAction::Ignore,
                "vk {vk:#04x} should be ignored"
            );
        }
    }

    #[test]
    fn letters_digits_and_the_dot_are_decode_candidates() {
        // The characters a trigger is actually made of must reach the decoder, not be swallowed.
        for vk in [
            b'A' as u16, // 0x41
            b'Z' as u16,
            b'0' as u16,
            b'9' as u16,
            0xBEu16, // VK_OEM_PERIOD — the '.' that starts every dot-trigger
            0x20,    // VK_SPACE
        ] {
            assert_eq!(
                key_action(vk),
                KeyAction::Decode,
                "vk {vk:#04x} should decode"
            );
        }
    }
}
