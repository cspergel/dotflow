//! DotFlow — spoken-punctuation command table (design §12, Mode B "spoken").
//!
//! A pure, deterministic text transform: the user dictates punctuation explicitly ("period", "new
//! paragraph", "open paren") and this maps those spoken tokens to marks, stripping the token from the
//! text. This is the sub-1ms deterministic layer (no LLM) the DotFlow design specifies — the same
//! command-table pattern macparakeet's "Voice Return" uses.
//!
//! Kept pure + total (no IO, no clock, no global state) so it is unit-testable AND verifiable by the
//! DTF referee. The three punctuation MODES (auto / spoken / raw) are selected upstream; this module
//! implements the SPOKEN mapping only.

/// The three punctuation modes (design §12): `Auto` = keep the model's native punctuation; `Spoken` =
/// strip and apply the spoken command table (this module); `Raw` = no punctuation processing at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PunctuationMode {
    Auto,
    Spoken,
    Raw,
}

/// A spoken token → its literal mark. `attaches_left` = the mark hugs the previous word (no leading
/// space): "period" → "." lands as `word.`, whereas "open paren" → "(" leads the NEXT word: `word (`.
struct Mark {
    spoken: &'static str,
    mark: &'static str,
    attaches_left: bool,
}

/// The deterministic command table. Longest phrases FIRST so multi-word tokens ("new paragraph") match
/// before their prefixes ("new"). Verbatim, ASCII, locale-independent (design: no OS-locale dependence).
const MARKS: &[Mark] = &[
    Mark {
        spoken: "new paragraph",
        mark: "\n\n",
        attaches_left: true,
    },
    Mark {
        spoken: "new line",
        mark: "\n",
        attaches_left: true,
    },
    Mark {
        spoken: "question mark",
        mark: "?",
        attaches_left: true,
    },
    Mark {
        spoken: "exclamation point",
        mark: "!",
        attaches_left: true,
    },
    Mark {
        spoken: "exclamation mark",
        mark: "!",
        attaches_left: true,
    },
    Mark {
        spoken: "open paren",
        mark: "(",
        attaches_left: false,
    },
    Mark {
        spoken: "close paren",
        mark: ")",
        attaches_left: true,
    },
    Mark {
        spoken: "semicolon",
        mark: ";",
        attaches_left: true,
    },
    Mark {
        spoken: "colon",
        mark: ":",
        attaches_left: true,
    },
    Mark {
        spoken: "comma",
        mark: ",",
        attaches_left: true,
    },
    Mark {
        spoken: "period",
        mark: ".",
        attaches_left: true,
    },
    Mark {
        spoken: "full stop",
        mark: ".",
        attaches_left: true,
    },
];

/// Apply the spoken-punctuation command table to `input`. Whole-token match, case-insensitive on the
/// spoken word(s); the surrounding words keep their original text. Deterministic + total.
///
/// Example: `apply_spoken("the patient is stable period new paragraph plan colon admit")`
///        → `"the patient is stable.\n\nplan: admit"`.
pub fn apply_spoken(input: &str) -> String {
    // Tokenize on ASCII whitespace; we re-emit spacing ourselves so mark attachment is exact.
    let tokens: Vec<&str> = input.split_whitespace().collect();
    let mut out = String::with_capacity(input.len());
    let mut i = 0;
    while i < tokens.len() {
        if let Some((mark, consumed)) = match_mark(&tokens[i..]) {
            if mark.attaches_left {
                out.push_str(mark.mark); // hug the previous word: "stable" + "." = "stable."
            } else {
                push_spaced(&mut out, mark.mark); // lead the next word: "... ("
            }
            i += consumed;
        } else {
            push_spaced(&mut out, tokens[i]);
            i += 1;
        }
    }
    out
}

/// Longest-match a spoken mark at the start of `rest`. Returns the mark + how many tokens it consumed.
fn match_mark(rest: &[&str]) -> Option<(&'static Mark, usize)> {
    for m in MARKS {
        let spoken_words: Vec<&str> = m.spoken.split(' ').collect();
        let n = spoken_words.len();
        if rest.len() >= n
            && rest[..n]
                .iter()
                .zip(&spoken_words)
                .all(|(a, b)| a.eq_ignore_ascii_case(b))
        {
            return Some((m, n));
        }
    }
    None
}

/// Append `s` with a single separating space unless `out` is empty or already ends in an open bracket
/// or a newline (so "(" hugs the following word and a new paragraph doesn't get a leading space).
fn push_spaced(out: &mut String, s: &str) {
    let needs_space = !out.is_empty() && !out.ends_with('(') && !out.ends_with('\n');
    if needs_space {
        out.push(' ');
    }
    out.push_str(s);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn period_hugs_the_previous_word() {
        assert_eq!(
            apply_spoken("the patient is stable period"),
            "the patient is stable."
        );
    }

    #[test]
    fn multi_word_marks_match_before_prefixes() {
        // "new paragraph" must win over "new" + "paragraph", and "new line" over "new".
        assert_eq!(
            apply_spoken("done period new paragraph next"),
            "done.\n\nnext"
        );
        assert_eq!(
            apply_spoken("line one new line line two"),
            "line one\nline two"
        );
    }

    #[test]
    fn case_insensitive_spoken_tokens() {
        assert_eq!(apply_spoken("wait Comma then go Period"), "wait, then go.");
    }

    #[test]
    fn open_paren_leads_the_next_word_close_paren_hugs() {
        assert_eq!(
            apply_spoken("the dose open paren ten mg close paren daily"),
            "the dose (ten mg) daily"
        );
    }

    #[test]
    fn colon_and_semicolon_and_question() {
        assert_eq!(apply_spoken("plan colon admit"), "plan: admit");
        assert_eq!(apply_spoken("a semicolon b"), "a; b");
        assert_eq!(apply_spoken("really question mark"), "really?");
    }

    #[test]
    fn a_word_that_merely_contains_a_mark_word_is_not_a_mark() {
        // "commanding" is not "comma"; only whole tokens match.
        assert_eq!(
            apply_spoken("the commanding officer"),
            "the commanding officer"
        );
    }

    #[test]
    fn empty_and_no_marks_are_total_and_identity_ish() {
        assert_eq!(apply_spoken(""), "");
        assert_eq!(apply_spoken("just plain words"), "just plain words");
    }

    #[test]
    fn full_stop_is_an_alias_for_period() {
        assert_eq!(apply_spoken("done full stop"), "done.");
    }
}
