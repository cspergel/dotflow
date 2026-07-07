//! DotFlow — dot-phrase / voice-alias expansion (design §4 "Expand", §11c "parse-and-expand BEFORE inject").
//!
//! The wedge: the user types or says a shortcut and their reusable text lands in the field.
//!   - **Dot-trigger** (`.copd`): a whole token starting with `.` expands to its template.
//!   - **Voice-alias** ("insert copd plan"): a spoken command phrase expands to the same template.
//!
//! Pure + deterministic + total (no IO/clock/global state) — unit-testable and DTF-verifiable. The
//! phrase LIBRARY (SQLite, add/edit UI, search palette, import/export) is the effectful shell around this
//! core; this module is only the expansion FUNCTION over a supplied table.

use std::collections::HashMap;

/// One reusable phrase: a dot trigger, zero+ spoken aliases, and the expansion it produces.
#[derive(Debug, Clone)]
pub struct Phrase {
    /// The dot trigger WITHOUT the leading dot, lowercased (e.g. `"copd"` for `.copd`).
    pub key: String,
    /// Spoken aliases, lowercased (e.g. `"insert copd plan"`). Matched case-insensitively as whole phrases.
    pub aliases: Vec<String>,
    /// The text this phrase expands to.
    pub expansion: String,
}

/// A compiled lookup: dot-key → expansion, and each alias as its CANONICAL word list + expansion. Built
/// once from the phrase list. Alias matching is over canonical words (see `canonical_words`), so the ASR's
/// hyphenation/capitalization/punctuation ("Insert follow-up.") still matches the spoken trigger.
#[derive(Debug, Default)]
pub struct PhraseTable {
    by_key: HashMap<String, String>,
    /// (canonical alias words, expansion), sorted LONGEST-first so "insert copd plan" wins over "insert copd".
    aliases: Vec<(Vec<String>, String)>,
}

impl PhraseTable {
    pub fn new(phrases: &[Phrase]) -> Self {
        let mut t = PhraseTable::default();
        for p in phrases {
            t.by_key.insert(norm(&p.key), p.expansion.clone());
            for a in &p.aliases {
                let words = canonical_words(a);
                if !words.is_empty() {
                    t.aliases.push((words, p.expansion.clone()));
                }
            }
        }
        // Longest alias (by canonical word count) first, so the greediest trigger matches.
        t.aliases.sort_by(|a, b| b.0.len().cmp(&a.0.len()));
        t
    }

    fn dot(&self, key_lower: &str) -> Option<&str> {
        self.by_key.get(key_lower).map(|s| s.as_str())
    }

    /// True iff the CANONICAL words of `tokens` form a PROPER prefix of some alias's canonical words — i.e.
    /// an alias exists that has MORE words and begins with exactly these. The streaming command-buffer uses
    /// this to HOLD a spoken trigger still being said ("insert copd" → wait for "plan") so a multi-word
    /// phrase lands as one clean block. A complete match (equal length) is NOT a proper prefix — that alias
    /// is ready to expand, not hold.
    pub fn is_partial_alias(&self, tokens: &[&str]) -> bool {
        let input: Vec<String> = tokens.iter().flat_map(|t| canonical_words(t)).collect();
        if input.is_empty() {
            return false;
        }
        self.aliases.iter().any(|(seq, _)| {
            seq.len() > input.len() && seq.iter().zip(input.iter()).all(|(s, i)| s == i)
        })
    }
}

/// Expand dot-triggers and voice-aliases in `input` against `table`. Non-trigger text is unchanged.
/// Deterministic + total; an unknown `.foo` is left verbatim (it is not a known phrase — never guessed).
///
/// Ordering (design §11c): a segment that IS a command resolves to one clean block. Here we scan
/// left-to-right, preferring a longest voice-alias match, else a dot-trigger token, else pass the word.
pub fn expand(input: &str, table: &PhraseTable) -> String {
    let tokens: Vec<&str> = input.split_whitespace().collect();
    let mut out = String::with_capacity(input.len());
    let mut i = 0;
    while i < tokens.len() {
        // 1) longest voice-alias match starting at i
        if let Some((expansion, consumed)) = match_alias(&tokens[i..], table) {
            push_spaced(&mut out, expansion);
            i += consumed;
            continue;
        }
        // 2) a dot-trigger token (".copd"): strip the dot, look up; unknown ⇒ leave the token verbatim.
        let tok = tokens[i];
        if let Some(key) = tok.strip_prefix('.') {
            if let Some(expansion) = table.dot(&norm(key)) {
                push_spaced(&mut out, expansion);
                i += 1;
                continue;
            }
        }
        // 3) ordinary word
        push_spaced(&mut out, tok);
        i += 1;
    }
    out
}

/// Normalize a spoken token for TRIGGER MATCHING ONLY (never for passthrough output): lowercase and drop
/// any ASCII punctuation the ASR attaches ("Up." → "up", "insert," → "insert"). Non-trigger dictation is
/// always emitted from the ORIGINAL token, so its real capitalization + punctuation are preserved.
pub(crate) fn norm(tok: &str) -> String {
    tok.trim_matches(|c: char| c.is_ascii_punctuation()).to_lowercase()
}

/// Split a token into its CANONICAL words for matching: break on hyphens (so the ASR's "follow-up" matches
/// the spoken "follow up") and whitespace, lowercase, and strip surrounding punctuation. Empty pieces drop.
/// Matching-only — passthrough always emits the ORIGINAL token, so real hyphens/case survive when no trigger
/// fires.
pub(crate) fn canonical_words(tok: &str) -> Vec<String> {
    tok.split(|c: char| c == '-' || c.is_whitespace())
        .map(norm)
        .filter(|s| !s.is_empty())
        .collect()
}

/// Longest voice-alias match at the start of `rest` (original tokens). Accumulates the CANONICAL words of
/// successive tokens until they exactly equal an alias's canonical words at a token boundary, then reports
/// how many ORIGINAL tokens that consumed. Because one token can supply several canonical words ("follow-up"
/// → follow, up), an N-word alias may be satisfied by fewer original tokens.
fn match_alias<'t>(rest: &[&str], table: &'t PhraseTable) -> Option<(&'t str, usize)> {
    for (alias_words, expansion) in &table.aliases {
        let mut acc: Vec<String> = Vec::new();
        let mut consumed = 0usize;
        for &tok in rest {
            acc.extend(canonical_words(tok));
            consumed += 1;
            if acc.len() >= alias_words.len() {
                break;
            }
        }
        // Exact, token-aligned match only (an overshoot means the last token spilled past the alias — not a
        // clean trigger, so we don't partially consume it).
        if consumed > 0 && acc == *alias_words {
            return Some((expansion.as_str(), consumed));
        }
    }
    None
}

fn push_spaced(out: &mut String, s: &str) {
    if !out.is_empty() && !out.ends_with('\n') {
        out.push(' ');
    }
    out.push_str(s);
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn dot_trigger_expands() {
        assert_eq!(expand(".copd", &table()), "COPD: continue inhalers, follow up pulmonary.");
    }

    #[test]
    fn dot_trigger_inside_a_sentence_expands_in_place() {
        assert_eq!(
            expand("assessment .fu thanks", &table()),
            "assessment Follow up in two weeks. thanks"
        );
    }

    #[test]
    fn voice_alias_expands_the_same_as_the_dot_trigger() {
        assert_eq!(
            expand("insert copd plan", &table()),
            expand(".copd", &table())
        );
    }

    #[test]
    fn longest_alias_wins_and_is_case_insensitive() {
        // "INSERT COPD PLAN" (upper) still matches the alias as one block, not word-by-word.
        assert_eq!(expand("INSERT COPD PLAN", &table()), "COPD: continue inhalers, follow up pulmonary.");
    }

    #[test]
    fn unknown_dot_trigger_is_left_verbatim_never_guessed() {
        assert_eq!(expand(".unknown here", &table()), ".unknown here");
    }

    #[test]
    fn plain_dictation_is_unchanged_and_total_on_empty() {
        assert_eq!(expand("the patient is doing well", &table()), "the patient is doing well");
        assert_eq!(expand("", &table()), "");
    }

    #[test]
    fn a_bare_dot_or_a_non_trigger_dotted_word_is_safe() {
        // "3.5" is not a phrase key; leave it. A lone "." isn't a known key either.
        assert_eq!(expand("dose 3.5 mg", &table()), "dose 3.5 mg");
    }

    #[test]
    fn asr_capitalization_and_trailing_punctuation_still_trigger() {
        // Real Parakeet output capitalizes and punctuates: "Insert follow up." must still expand.
        let exp = "Follow up in two weeks.";
        assert_eq!(expand("Insert follow up.", &table()), exp);
        assert_eq!(expand("insert follow up,", &table()), exp);
        // and mid-sentence with the ASR's comma on the last trigger word.
        assert_eq!(expand("note: insert follow up. thanks", &table()),
            format!("note: {} thanks", exp));
    }

    #[test]
    fn dot_trigger_with_attached_punctuation_still_expands() {
        assert_eq!(expand(".fu.", &table()), "Follow up in two weeks.");
    }

    #[test]
    fn a_non_trigger_word_keeps_its_own_punctuation_and_case() {
        // normalization is for MATCHING only — passthrough text is emitted verbatim.
        assert_eq!(expand("The Patient, stable.", &table()), "The Patient, stable.");
    }

    #[test]
    fn asr_hyphenation_still_triggers_the_spoken_phrase() {
        // Parakeet writes "insert follow up" as "Insert follow-up." — one hyphenated token — but it must
        // still fire the 3-word alias "insert follow up".
        assert_eq!(expand("Insert follow-up.", &table()), "Follow up in two weeks.");
        assert_eq!(expand("Insert follow-up", &table()), "Follow up in two weeks.");
        // mid-sentence too.
        assert_eq!(expand("note insert follow-up thanks", &table()), "note Follow up in two weeks. thanks");
    }

    #[test]
    fn a_non_trigger_hyphenated_word_keeps_its_hyphen() {
        // hyphen-splitting is matching-only; non-trigger passthrough keeps the original token verbatim.
        assert_eq!(expand("state-of-the-art care", &table()), "state-of-the-art care");
    }

    #[test]
    fn command_buffer_holds_a_hyphenated_partial_trigger() {
        // "insert follow-up" is a COMPLETE trigger (canonical insert/follow/up) → not a proper prefix.
        assert!(!table().is_partial_alias(&["insert", "follow-up"]));
        // but "insert" alone is still a growing trigger.
        assert!(table().is_partial_alias(&["insert"]));
    }
}
