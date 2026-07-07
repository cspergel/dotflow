//! DotFlow — deterministic text cleanup for the "clean up selected text" hotkey.
//!
//! This is the ZERO-SETUP fallback: mechanical fixes (whitespace, spacing around punctuation, sentence
//! capitalization, the pronoun "i") that need no model or network. When a post-process LLM is configured the
//! hotkey uses THAT instead for a fuller grammar/spelling cleanup; this runs when no LLM is available.
//!
//! Deliberately conservative — it must never corrupt text. It does not touch spelling, word choice, or
//! anything ambiguous (e.g. it leaves decimals and existing capitalization of non-sentence-start words alone).
//! Pure + total, so it is unit-testable.

/// Apply the mechanical cleanup. Returns the cleaned string (may equal the input if nothing needed fixing).
pub fn deterministic_cleanup(text: &str) -> String {
    // 1) Whitespace: within each line collapse runs of spaces/tabs to one and trim; preserve line breaks;
    //    drop leading/trailing blank lines.
    let mut s = text
        .lines()
        .map(|line| line.split_whitespace().collect::<Vec<_>>().join(" "))
        .collect::<Vec<_>>()
        .join("\n");
    s = s.trim().to_string();

    // 2) Remove a stray space before closing punctuation ("word ," -> "word,"). Period included: a real
    //    decimal ("3.5") has no surrounding spaces, so it is unaffected.
    for pat in [" ,", " ;", " :", " !", " ?", " .", " )"] {
        s = s.replace(pat, &pat[1..]);
    }

    // 3) Ensure a space AFTER "," ";" ":" "!" "?" when it directly precedes a LETTER ("wait,what" ->
    //    "wait, what"). Only letters, so number groupings ("3,000"), times ("3:30"), and URLs ("http://")
    //    are left intact.
    s = add_space_after_punct(&s);

    // 4) Capitalize sentence starts and the standalone pronoun "i".
    capitalize(&s)
}

fn add_space_after_punct(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len() + 8);
    for i in 0..chars.len() {
        let c = chars[i];
        out.push(c);
        if matches!(c, ',' | ';' | ':' | '!' | '?')
            && chars.get(i + 1).is_some_and(|n| n.is_alphabetic())
        {
            out.push(' ');
        }
    }
    out
}

/// A word "boundary" character — what can sit next to a standalone `i`.
fn is_boundary(c: char) -> bool {
    c.is_whitespace() || matches!(c, '(' | '"' | '\u{201C}' | '\u{2018}')
}

fn capitalize(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    // The next ALPHABETIC character begins a sentence and should be uppercased. True at the very start.
    let mut cap_next = true;

    for i in 0..chars.len() {
        let c = chars[i];

        // Standalone pronoun "i" → "I": an `i` whose left side is the start or a boundary, and whose right
        // side is the end, whitespace, closing punctuation, or an apostrophe (contractions: i'm, i'll, i've).
        let left_ok = i == 0 || is_boundary(chars[i - 1]);
        let right_ok = i + 1 >= chars.len() || {
            let n = chars[i + 1];
            n.is_whitespace()
                    || matches!(n, '.' | ',' | '!' | '?' | ';' | ':' | ')' | '"')
                    || n == '\'' // straight apostrophe
                    || n == '\u{2019}' // typographic apostrophe
        };
        if c == 'i' && left_ok && right_ok {
            out.push('I');
            cap_next = false;
            continue;
        }

        if cap_next && c.is_alphabetic() {
            out.extend(c.to_uppercase());
            cap_next = false;
        } else {
            out.push(c);
        }

        // Arm capitalization for the next sentence: at a new line, or after a terminator that actually ends a
        // token (followed by whitespace or the end). The "followed by whitespace/end" guard is what keeps a
        // decimal point ("3.5") or a dotted abbreviation from capitalizing the next word.
        if c == '\n'
            || (matches!(c, '.' | '!' | '?') && chars.get(i + 1).is_none_or(|n| n.is_whitespace()))
        {
            cap_next = true;
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_is_empty() {
        assert_eq!(deterministic_cleanup(""), "");
        assert_eq!(deterministic_cleanup("   \n  \t "), "");
    }

    #[test]
    fn already_clean_text_is_unchanged() {
        // A realness check: clean input must survive untouched (no spurious edits).
        let clean = "The patient is stable. Follow up in two weeks.";
        assert_eq!(deterministic_cleanup(clean), clean);
    }

    #[test]
    fn collapses_runs_of_spaces_and_trims() {
        assert_eq!(deterministic_cleanup("  hello    world  "), "Hello world");
    }

    #[test]
    fn removes_space_before_punctuation() {
        assert_eq!(
            deterministic_cleanup("Wait , what ? really !"),
            "Wait, what? Really!"
        );
    }

    #[test]
    fn capitalizes_sentence_starts() {
        assert_eq!(
            deterministic_cleanup("hello there. how are you? fine. ok"),
            "Hello there. How are you? Fine. Ok"
        );
    }

    #[test]
    fn capitalizes_standalone_i_and_contractions() {
        assert_eq!(
            deterministic_cleanup("i think i'm right and i'll stay"),
            "I think I'm right and I'll stay"
        );
    }

    #[test]
    fn does_not_capitalize_i_inside_a_word() {
        // "in", "it", "his" contain 'i' but must be left alone.
        assert_eq!(
            deterministic_cleanup("it is in his kit"),
            "It is in his kit"
        );
    }

    #[test]
    fn preserves_decimals() {
        // No spaces around the dot, so it is not treated as sentence punctuation or space-before-period.
        assert_eq!(
            deterministic_cleanup("give 3.5 mg twice"),
            "Give 3.5 mg twice"
        );
    }

    #[test]
    fn preserves_line_breaks() {
        assert_eq!(
            deterministic_cleanup("first line\nsecond line"),
            "First line\nSecond line"
        );
    }

    #[test]
    fn a_messy_sentence_is_cleaned() {
        assert_eq!(
            deterministic_cleanup("  i went home ,then i slept .  it was  late "),
            "I went home, then I slept. It was late"
        );
    }
}
