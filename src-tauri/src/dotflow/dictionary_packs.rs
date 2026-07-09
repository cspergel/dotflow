//! DotFlow — toggleable clinical/medical (and future user) dictionary packs for the Harper cleanup engine.
//!
//! Purpose: extend Harper's vocabulary so valid domain terms (drug names, standard abbreviations, anatomy,
//! specialty words) are **not flagged as misspellings** in the cleanup / review / selection-overlay flows —
//! without silently auto-correcting a typo *into* a drug name (the wrong-drug risk).
//!
//! Design: `docs/plans/2026-07-08-medical-dictionary-pack-design.md`. The hardening folds
//! (`[SWEEP-Fn]`, `[RS2-Fn]`, `[LOAD-Fn]`) are authoritative and are implemented here:
//!
//! - **Bundled default, always-current ([RS2-F3]).** The `medical` pack is compiled into the binary via
//!   `include_str!` — never seeded as an editable file, so a shipped update always carries the latest list.
//!   The runtime dictionaries dir (`%APPDATA%/…/dictionaries/*.txt`) is for **additional/user** packs only.
//! - **Per-pack build isolation ([SWEEP-F7] / [RS2-F4]).** Each enabled pack builds its own `FstDictionary`
//!   inside its own `catch_unwind` and is added individually to a [`MergedDictionary`]; a bad pack degrades
//!   only itself (fall back to curated + the packs that did build, log, never panic/wedge).
//! - **Build outside the lock ([SWEEP-F7]).** [`set_enabled_packs`] builds into a local then swaps the cached
//!   `Arc` under the lock; [`current_dictionary`]/[`current_snapshot`] only clone the cached `Arc`s.
//! - **Untrusted-file robustness ([RS2-F2] / [RS2-F7] / [LOAD-F-iso]).** Size cap is **metadata-gated** before
//!   reading; files must be regular files (symlinks not followed); a leading BOM is stripped; content is read
//!   UTF-8-lossy; file-count / per-file-term / aggregate-term caps bound memory.
//! - **Medical-jargon set for the safety filter ([RS2-F1]).** The set handed to `harper_cleanup`'s drop
//!   filter is **enabled-pack terms MINUS Harper's curated dictionary**, Unicode-lowercased — so pure jargon
//!   (`metoprolol`) is guarded but homographs (`cold`, `stroke`) that are real English words keep their
//!   normal corrections.
//! - **Reload forces a rebuild ([RS2-F5]).** `set_enabled_packs` always rebuilds, so editing an enabled
//!   pack's contents and reloading takes effect.

use std::collections::HashSet;
use std::panic::AssertUnwindSafe;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use harper_core::spell::{Dictionary, FstDictionary, MergedDictionary, MutableDictionary};
use harper_core::DictWordMetadata;
use log::warn;
use serde::{Deserialize, Serialize};
use specta::Type;

/// The bundled, always-current default pack id. Also its filename stem for de-dup against user files.
pub const MEDICAL_PACK_ID: &str = "medical";

/// The user's editable custom pack id (filename stem `custom.txt`). Words added in-app land here and flow
/// through the exact same acceptance-only safety filter as any other pack (jargon = terms MINUS curated).
pub const CUSTOM_PACK_ID: &str = "custom";

/// The bundled medical term list, compiled into the binary ([RS2-F3] — not seeded as an editable file).
const MEDICAL_PACK_CONTENT: &str = include_str!("../../resources/dictionaries/medical.txt");

/// Per-file byte cap, checked via metadata BEFORE reading ([RS2-F2]).
const SIZE_CAP_BYTES: u64 = 8 * 1024 * 1024;
/// Per-file term cap (parse stops after this many terms).
const TERM_CAP: usize = 500_000;
/// Max discovered `.txt` files to admit as packs ([RS2-F6]).
const FILE_COUNT_CAP: usize = 64;
/// Aggregate cap on jargon-set terms across all enabled packs ([RS2-F6]).
const AGGREGATE_TERM_CAP: usize = 1_000_000;

/// Where a pack's terms come from: the compiled-in default, or a discovered `.txt` file.
#[derive(Debug, Clone)]
enum PackSource {
    Bundled(&'static str),
    File(PathBuf),
}

/// One dictionary pack (the registry unit). `id` = lowercased filename stem (`medical.txt` → `medical`).
#[derive(Debug, Clone)]
pub struct DictionaryPack {
    pub id: String,
    pub label: String,
    source: PackSource,
}

/// Frontend-facing pack row: id, resolved label, whether it is enabled, and its term count.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct DictionaryPackInfo {
    pub id: String,
    pub label: String,
    pub enabled: bool,
    pub term_count: usize,
}

/// The process-wide cached, built dictionary: the merged (curated + enabled packs) [`Dictionary`] plus the
/// Unicode-lowercased medical-jargon set used by the silent-auto-fix safety filter. Built together so the
/// two are always consistent with the same enabled-pack snapshot.
struct CachedDict {
    dict: Arc<MergedDictionary>,
    jargon: Arc<HashSet<String>>,
}

fn cache() -> &'static Mutex<Option<CachedDict>> {
    static C: OnceLock<Mutex<Option<CachedDict>>> = OnceLock::new();
    C.get_or_init(|| Mutex::new(None))
}

/// The bundled medical pack (always present in the registry, regardless of the dictionaries dir).
fn bundled_medical() -> DictionaryPack {
    DictionaryPack {
        id: MEDICAL_PACK_ID.to_string(),
        label: "Medical".to_string(),
        source: PackSource::Bundled(MEDICAL_PACK_CONTENT),
    }
}

/// Titlecase a lowercased pack id for display (`legal` → `Legal`).
fn titlecase(id: &str) -> String {
    let mut chars = id.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

/// Discover every pack: the bundled default first, then each regular `*.txt` file in `dir`. Non-`.txt`
/// files, dotfiles, subdirs, and symlinks are ignored; a file whose id collides with an already-seen id
/// (e.g. a user `medical.txt`) is skipped so the bundled default always wins ([RS2-F3]). File-count capped.
pub fn discover_packs(dir: &Path) -> Vec<DictionaryPack> {
    let mut packs = vec![bundled_medical()];
    let mut seen: HashSet<String> = HashSet::from([MEDICAL_PACK_ID.to_string()]);

    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return packs, // no dir yet (or unreadable) → bundled default only
    };

    for entry in entries.flatten() {
        if packs.len() >= FILE_COUNT_CAP + 1 {
            warn!("dictionary_packs: file-count cap ({FILE_COUNT_CAP}) reached; ignoring the rest");
            break;
        }
        let path = entry.path();

        // Regular files only — do NOT follow symlinks ([RS2-F7]). symlink_metadata does not traverse.
        let meta = match std::fs::symlink_metadata(&path) {
            Ok(m) => m,
            Err(_) => continue,
        };
        if !meta.file_type().is_file() {
            continue;
        }

        // Extension must be `.txt` (case-insensitive); skip dotfiles.
        let is_txt = path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("txt"));
        if !is_txt {
            continue;
        }
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if name.starts_with('.') {
            continue;
        }
        let stem = match path.file_stem().and_then(|s| s.to_str()) {
            Some(s) if !s.is_empty() => s,
            _ => continue,
        };
        let id = stem.to_lowercase();
        if !seen.insert(id.clone()) {
            warn!("dictionary_packs: duplicate pack id '{id}' ({name}) skipped");
            continue;
        }
        let label = titlecase(&id);
        packs.push(DictionaryPack {
            id,
            label,
            source: PackSource::File(path),
        });
    }

    packs
}

/// Read a pack's raw text with all untrusted-file guards. `None` = skip this pack (logged upstream by the
/// build). Metadata-gates the size cap BEFORE reading ([RS2-F2]); regular-files-only, no symlink follow
/// ([RS2-F7]); UTF-8-lossy; strips a leading BOM ([RS2-F7]).
fn read_pack_content(pack: &DictionaryPack) -> Option<String> {
    match &pack.source {
        PackSource::Bundled(s) => Some((*s).to_string()),
        PackSource::File(path) => {
            let meta = std::fs::symlink_metadata(path).ok()?;
            if !meta.file_type().is_file() {
                warn!(
                    "dictionary_packs: '{}' is not a regular file; skipped",
                    path.display()
                );
                return None;
            }
            if meta.len() > SIZE_CAP_BYTES {
                warn!(
                    "dictionary_packs: '{}' exceeds size cap ({} bytes); skipped",
                    path.display(),
                    SIZE_CAP_BYTES
                );
                return None;
            }
            let bytes = std::fs::read(path).ok()?;
            let mut s = String::from_utf8_lossy(&bytes).into_owned();
            if s.starts_with('\u{feff}') {
                s.remove(0); // strip UTF-8 BOM
            }
            Some(s)
        }
    }
}

/// Parse a pack's terms: one per line; blank lines and `#` comment lines skipped; leading/trailing
/// whitespace trimmed. Capped at [`TERM_CAP`].
fn parse_terms(content: &str) -> Vec<String> {
    let mut terms = Vec::new();
    for line in content.lines() {
        let t = line.trim();
        if t.is_empty() || t.starts_with('#') {
            continue;
        }
        terms.push(t.to_string());
        if terms.len() >= TERM_CAP {
            warn!("dictionary_packs: term cap ({TERM_CAP}) reached; truncating pack");
            break;
        }
    }
    terms
}

/// Resolve a pack's display label: an optional leading `# label: …` directive overrides the derived label.
/// Only comment lines before the first term are inspected.
fn parse_label(content: &str, fallback: &str) -> String {
    for line in content.lines() {
        let t = line.trim();
        if t.is_empty() {
            continue;
        }
        let Some(rest) = t.strip_prefix('#') else {
            break; // first real (non-comment) line — stop looking
        };
        let rest = rest.trim();
        if let Some(idx) = rest.find(':') {
            let (key, val) = rest.split_at(idx);
            if key.trim().eq_ignore_ascii_case("label") {
                let val = val[1..].trim();
                if !val.is_empty() {
                    return val.to_string();
                }
            }
        }
    }
    fallback.to_string()
}

/// Build one pack's `FstDictionary` from its file/bundled content. `None` = skip (unreadable, over-cap, or
/// empty — an empty pack behaves as pack-off, [SWEEP-F4]). Returns the FST plus the raw term list (for the
/// jargon set). Called INSIDE a `catch_unwind` by [`build`] so a panic degrades only this pack.
fn build_pack_fst(pack: &DictionaryPack) -> Option<(Arc<FstDictionary>, Vec<String>)> {
    let content = read_pack_content(pack)?;
    let terms = parse_terms(&content);
    if terms.is_empty() {
        return None;
    }
    let mut md = MutableDictionary::new();
    for t in &terms {
        // Default metadata is enough to count as valid vocabulary and suppress the Spelling lint; its empty
        // DialectFlags passes the dialect gate (verified in the design sweep). No POS metadata is needed.
        md.append_word_str(t, DictWordMetadata::default());
    }
    let fst: FstDictionary = md.into();
    Some((Arc::new(fst), terms))
}

/// Build the merged dictionary + jargon set for the given enabled ids. Curated is always included; each
/// enabled pack is folded in independently inside its own `catch_unwind` ([SWEEP-F7] / [RS2-F4]) so a bad
/// pack degrades only itself. Pure w.r.t. process state — does not touch the cache (the caller swaps).
fn build(dir: &Path, enabled_ids: &[String]) -> CachedDict {
    let packs = discover_packs(dir);
    let curated: Arc<FstDictionary> = FstDictionary::curated();

    let mut merged = MergedDictionary::new();
    merged.add_dictionary(curated.clone());

    let mut jargon: HashSet<String> = HashSet::new();
    let mut aggregate = 0usize;

    for pack in &packs {
        if !enabled_ids.iter().any(|e| e == &pack.id) {
            continue;
        }
        let built = std::panic::catch_unwind(AssertUnwindSafe(|| build_pack_fst(pack)))
            .unwrap_or_else(|_| {
                warn!(
                    "dictionary_packs: pack '{}' panicked while building; skipped",
                    pack.id
                );
                None
            });
        let Some((fst, terms)) = built else {
            continue;
        };
        merged.add_dictionary(fst);

        // Jargon set = enabled-pack terms MINUS curated, Unicode-lowercased ([RS2-F1]).
        for t in terms {
            if aggregate >= AGGREGATE_TERM_CAP {
                warn!("dictionary_packs: aggregate term cap reached; jargon set truncated");
                break;
            }
            aggregate += 1;
            let lower = t.to_lowercase();
            if !curated.contains_word_str(&lower) {
                jargon.insert(lower);
            }
        }
    }

    CachedDict {
        dict: Arc::new(merged),
        jargon: Arc::new(jargon),
    }
}

/// Curated-only cache entry (no packs) — the default before [`set_enabled_packs`] runs, and the result of
/// enabling nothing. Cheap: curated is Harper's own cached `Arc`.
fn build_curated_only() -> CachedDict {
    let mut merged = MergedDictionary::new();
    merged.add_dictionary(FstDictionary::curated());
    CachedDict {
        dict: Arc::new(merged),
        jargon: Arc::new(HashSet::new()),
    }
}

/// Rebuild the process-wide cache for the given enabled pack ids and swap it in. Called at startup and on
/// every pack toggle / reload. ALWAYS rebuilds ([RS2-F5] — so editing an enabled pack's contents takes
/// effect on reload). Builds OUTSIDE the lock, then takes the lock only to swap the `Arc` ([SWEEP-F7]).
/// Never panics: a bad pack degrades to curated + the packs that built.
pub fn set_enabled_packs(dir: &Path, enabled_ids: &[String]) {
    let built = build(dir, enabled_ids); // outside the lock
    let mut guard = cache().lock().unwrap_or_else(|p| p.into_inner());
    *guard = Some(built);
}

/// Atomic snapshot of `(merged dictionary, medical-jargon set)` from the cache. Initializes to curated-only
/// on first use if [`set_enabled_packs`] has not run. Read path: only clones the cached `Arc`s.
pub fn current_snapshot() -> (Arc<MergedDictionary>, Arc<HashSet<String>>) {
    let mut guard = cache().lock().unwrap_or_else(|p| p.into_inner());
    if guard.is_none() {
        *guard = Some(build_curated_only());
    }
    let c = guard.as_ref().expect("cache populated above");
    (c.dict.clone(), c.jargon.clone())
}

/// The merged dictionary in effect (curated + enabled packs). Used by `grammar::analyze` and for wiring
/// tests. Cheap `Arc` clone.
pub fn current_dictionary() -> Arc<MergedDictionary> {
    current_snapshot().0
}

/// Build the frontend pack list: every discovered pack with its resolved label, enabled state, and term
/// count. Reads each pack's content (bounded by the size cap) to derive the label override + count.
pub fn pack_infos(dir: &Path, enabled_ids: &[String]) -> Vec<DictionaryPackInfo> {
    discover_packs(dir)
        .into_iter()
        .map(|p| {
            let (label, term_count) = match read_pack_content(&p) {
                Some(content) => (parse_label(&content, &p.label), parse_terms(&content).len()),
                None => (p.label.clone(), 0),
            };
            DictionaryPackInfo {
                enabled: enabled_ids.iter().any(|e| e == &p.id),
                id: p.id,
                label,
                term_count,
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------------------------------------
// User custom words ("My Words" pack). A thin, safe editor over `dictionaries/custom.txt` so users can add
// their own accepted vocabulary from inside the app. It is just another `.txt` pack (id `custom`), so it is
// discovered, toggled, and safety-filtered by all the machinery above — no special-casing in build/analyze.
// ---------------------------------------------------------------------------------------------------------

/// The header written atop `custom.txt` — the `# label:` directive names the pack "My Words" in the UI.
const CUSTOM_HEADER: &str = "# label: My Words\n# DotFlow custom dictionary — one accepted word per line. \
Edit here or from Settings → Dictionaries. Blank lines and lines starting with # are ignored.\n";

/// Path to the user's custom words file within the dictionaries dir.
pub fn custom_words_path(dir: &Path) -> PathBuf {
    dir.join("custom.txt")
}

/// Read the user's custom words (parsed, insertion-order-preserving). Empty if the file is absent, not a
/// regular file, over the size cap, or unreadable — same guards as any pack ([RS2-F2]/[RS2-F7]).
pub fn read_custom_words(dir: &Path) -> Vec<String> {
    let path = custom_words_path(dir);
    let Ok(meta) = std::fs::symlink_metadata(&path) else {
        return Vec::new();
    };
    if !meta.file_type().is_file() || meta.len() > SIZE_CAP_BYTES {
        return Vec::new();
    }
    let Ok(bytes) = std::fs::read(&path) else {
        return Vec::new();
    };
    let mut s = String::from_utf8_lossy(&bytes).into_owned();
    if s.starts_with('\u{feff}') {
        s.remove(0);
    }
    parse_terms(&s)
}

/// Sanitize a single custom word: trim, drop control / quote / angle-bracket chars, then reject empty,
/// whitespace-containing (Harper spell-checks per token, so multi-word entries can't match), or over-long
/// input. Returns the cleaned word or a user-facing error message.
pub fn sanitize_custom_word(raw: &str) -> Result<String, String> {
    let cleaned: String = raw
        .trim()
        .chars()
        .filter(|c| !c.is_control() && !matches!(c, '<' | '>' | '"' | '\''))
        .collect();
    let cleaned = cleaned.trim().to_string();
    if cleaned.is_empty() {
        return Err("Enter a word to add.".to_string());
    }
    if cleaned.chars().any(char::is_whitespace) {
        return Err("A custom word can't contain spaces — add one word at a time.".to_string());
    }
    if cleaned.chars().count() > 60 {
        return Err("That word is too long (max 60 characters).".to_string());
    }
    Ok(cleaned)
}

/// Write `custom.txt` (header + one word per line). Creates the dir if needed. Overwrites atomically enough
/// for a hand-edited user file (whole-file rewrite from the parsed list).
fn write_custom_words(dir: &Path, words: &[String]) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("create dictionaries dir: {e}"))?;
    let mut out = String::from(CUSTOM_HEADER);
    for w in words {
        out.push_str(w);
        out.push('\n');
    }
    std::fs::write(custom_words_path(dir), out).map_err(|e| format!("write custom.txt: {e}"))
}

/// Add a custom word (case-insensitive de-dup, insertion order). Returns the updated list.
pub fn add_custom_word(dir: &Path, raw: &str) -> Result<Vec<String>, String> {
    let word = sanitize_custom_word(raw)?;
    let mut words = read_custom_words(dir);
    let lower = word.to_lowercase();
    if !words.iter().any(|w| w.to_lowercase() == lower) {
        words.push(word);
    }
    write_custom_words(dir, &words)?;
    Ok(words)
}

/// Remove a custom word (case-insensitive). Returns the updated list.
pub fn remove_custom_word(dir: &Path, raw: &str) -> Result<Vec<String>, String> {
    let lower = raw.trim().to_lowercase();
    let mut words = read_custom_words(dir);
    words.retain(|w| w.to_lowercase() != lower);
    write_custom_words(dir, &words)?;
    Ok(words)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "dotflow-dict-{}-{}-{:?}",
            tag,
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    // Test 4 — registry drives the dict (wiring). enabled=["medical"] contains the term; enabled=[] does not.
    #[test]
    fn registry_drives_the_dictionary() {
        let dir = temp_dir("wiring");

        let on = build(&dir, &["medical".to_string()]);
        // NOTE: contains_word_str (the real trait method) — there is no `.contains()` on Dictionary.
        assert!(
            on.dict.contains_word_str("metoprolol"),
            "medical pack on → term is in the merged dict"
        );

        let off = build(&dir, &[]);
        assert!(
            !off.dict.contains_word_str("metoprolol"),
            "no packs → term absent from the merged dict"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    // Test 4 premise guard + jargon set — the jargon set is pack-terms MINUS curated ([RS2-F1]).
    #[test]
    fn jargon_set_excludes_curated_homographs() {
        // Premise: metoprolol is NOT in curated (pure jargon → belongs in the jargon set).
        assert!(
            !FstDictionary::curated().contains_word_str("metoprolol"),
            "premise: metoprolol must be absent from curated Harper"
        );
        // Premise: 'cold' IS a real English word in curated (a homograph → must be EXCLUDED from jargon).
        assert!(
            FstDictionary::curated().contains_word_str("cold"),
            "premise: 'cold' must be a curated English word"
        );

        let dir = temp_dir("jargon");
        // A pack that contains both pure jargon and a common-English homograph.
        std::fs::write(dir.join("clinic.txt"), "metoprolol\ncold\n").unwrap();

        let built = build(&dir, &["clinic".to_string()]);
        assert!(
            built.jargon.contains("metoprolol"),
            "pure jargon is in the drop-set"
        );
        assert!(
            !built.jargon.contains("cold"),
            "curated homograph is excluded from the drop-set"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    // Test 5 — pack-file parser boundaries.
    #[test]
    fn parser_skips_comments_blanks_and_trims() {
        let content =
            "# label: My Pack\n# heading comment\n\n  spacedterm  \nplainterm\nplainterm\n";
        let terms = parse_terms(content);
        assert!(
            terms.contains(&"spacedterm".to_string()),
            "trailing/leading ws trimmed"
        );
        assert!(terms.contains(&"plainterm".to_string()));
        assert!(
            !terms.iter().any(|t| t.contains("heading")),
            "comment lines must NOT become vocabulary"
        );
        assert!(
            !terms.iter().any(|t| t.contains("label")),
            "the label directive is a comment, not a term"
        );
        assert_eq!(
            parse_label(content, "Fallback"),
            "My Pack",
            "label override honored"
        );
    }

    #[test]
    fn empty_pack_behaves_as_pack_off_without_panic() {
        let dir = temp_dir("empty");
        std::fs::write(dir.join("blank.txt"), "# only a comment\n\n").unwrap();
        // Building an empty pack must not panic and must add nothing.
        let built = build(&dir, &["blank".to_string()]);
        assert!(built.jargon.is_empty(), "empty pack contributes no jargon");
        // Sanity: curated still works (a-vs-an will be exercised in grammar tests).
        assert!(built.dict.contains_word_str("the"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn duplicate_term_and_already_curated_term_do_not_panic() {
        let dir = temp_dir("dup");
        // Duplicate lines + a term already in curated ('tachycardia' is medical/absent, 'the' is curated).
        std::fs::write(dir.join("dup.txt"), "the\nthe\ntachycardia\ntachycardia\n").unwrap();
        let built = build(&dir, &["dup".to_string()]);
        // 'the' is curated → excluded from jargon; 'tachycardia' (absent from curated) → included once.
        assert!(!built.jargon.contains("the"));
        assert!(built.dict.contains_word_str("tachycardia"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    // Test 6 — discovery + user-pack ignore rules; bundled medical always present.
    #[test]
    fn discover_finds_txt_ignores_others_and_keeps_bundled_medical() {
        let dir = temp_dir("discover");
        std::fs::write(dir.join("legal.txt"), "estoppel\n").unwrap();
        std::fs::write(dir.join("notes.md"), "ignore me\n").unwrap();
        std::fs::write(dir.join(".hidden.txt"), "secret\n").unwrap();
        std::fs::create_dir_all(dir.join("subdir")).unwrap();

        let packs = discover_packs(&dir);
        let ids: HashSet<&str> = packs.iter().map(|p| p.id.as_str()).collect();
        assert!(ids.contains("medical"), "bundled default always present");
        assert!(ids.contains("legal"), "user .txt discovered");
        assert!(!ids.contains("notes"), "non-.txt ignored");
        assert!(!ids.contains(".hidden"), "dotfile ignored");
        assert!(!ids.contains("subdir"), "subdir ignored");

        // A user medical.txt must NOT shadow the bundled default (first wins).
        std::fs::write(dir.join("medical.txt"), "shadowsentinel\n").unwrap();
        let built = build(&dir, &["medical".to_string()]);
        assert!(
            built.dict.contains_word_str("metoprolol"),
            "bundled medical wins over a user medical.txt"
        );
        assert!(
            !built.dict.contains_word_str("shadowsentinel"),
            "the shadowing user file is skipped"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    // Test 7 — bad-file isolation has teeth: a good pack + an over-cap bad pack ⇒ good survives, bad skipped.
    #[test]
    fn bad_pack_is_isolated_good_pack_survives() {
        let dir = temp_dir("isolation");
        std::fs::write(dir.join("good.txt"), "goodclinicalword\n").unwrap();
        // A file larger than the size cap — must be skipped by the metadata gate, not read into memory.
        let big = vec![b'x'; (SIZE_CAP_BYTES + 1) as usize];
        std::fs::write(dir.join("bad.txt"), &big).unwrap();

        let built = build(&dir, &["good".to_string(), "bad".to_string()]);
        assert!(
            built.dict.contains_word_str("goodclinicalword"),
            "the good pack still contributes despite the bad one"
        );
        // The canary: if isolation were removed and the over-cap file were read+parsed, this would balloon
        // memory / include its junk. read_pack_content returns None for it, so nothing from it lands.
        assert!(
            built.jargon.contains("goodclinicalword"),
            "good pack's jargon present; bad pack silently skipped"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn oversize_file_is_rejected_by_metadata_gate() {
        let dir = temp_dir("oversize");
        let path = dir.join("huge.txt");
        std::fs::write(&path, vec![b'a'; (SIZE_CAP_BYTES + 10) as usize]).unwrap();
        let pack = DictionaryPack {
            id: "huge".to_string(),
            label: "Huge".to_string(),
            source: PackSource::File(path),
        };
        assert!(
            read_pack_content(&pack).is_none(),
            "over-cap file skipped before read"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn bom_is_stripped() {
        let dir = temp_dir("bom");
        let path = dir.join("bom.txt");
        std::fs::write(&path, "\u{feff}bomterm\n").unwrap();
        let pack = DictionaryPack {
            id: "bom".to_string(),
            label: "Bom".to_string(),
            source: PackSource::File(path),
        };
        let content = read_pack_content(&pack).unwrap();
        let terms = parse_terms(&content);
        assert_eq!(
            terms,
            vec!["bomterm".to_string()],
            "BOM stripped, term clean"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    // Test 8 — reload picks up a newly dropped file (discovery re-run, not cached from startup).
    #[test]
    fn reload_picks_up_a_newly_dropped_pack() {
        let dir = temp_dir("reload");
        // An invented token guaranteed absent from curated Harper.
        let term = "zzqreloadtoken";
        // Enable "legal" before any legal.txt exists — a tolerated no-op.
        let built0 = build(&dir, &["legal".to_string()]);
        assert!(!built0.dict.contains_word_str(term), "no legal.txt yet");

        // Drop the file, rebuild (the reload path) — the term is now accepted.
        std::fs::write(dir.join("legal.txt"), format!("{term}\n")).unwrap();
        let built1 = build(&dir, &["legal".to_string()]);
        assert!(
            built1.dict.contains_word_str(term),
            "reload re-runs discovery and picks up the dropped file"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    // [RS2-F5] — editing an already-enabled pack's contents takes effect on rebuild.
    #[test]
    fn editing_an_enabled_pack_takes_effect_on_rebuild() {
        let dir = temp_dir("edit");
        std::fs::write(dir.join("edit.txt"), "firstterm\n").unwrap();
        let a = build(&dir, &["edit".to_string()]);
        assert!(a.dict.contains_word_str("firstterm"));
        assert!(!a.dict.contains_word_str("secondterm"));

        std::fs::write(dir.join("edit.txt"), "firstterm\nsecondterm\n").unwrap();
        let b = build(&dir, &["edit".to_string()]);
        assert!(
            b.dict.contains_word_str("secondterm"),
            "edit picked up on rebuild"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn pack_infos_reports_label_override_and_count() {
        let dir = temp_dir("infos");
        std::fs::write(
            dir.join("legal.txt"),
            "# label: Legal Terminology\n# note\nestoppel\ntort\n",
        )
        .unwrap();
        let infos = pack_infos(&dir, &["legal".to_string()]);
        let legal = infos.iter().find(|i| i.id == "legal").unwrap();
        assert_eq!(legal.label, "Legal Terminology");
        assert_eq!(legal.term_count, 2);
        assert!(legal.enabled);
        let medical = infos.iter().find(|i| i.id == "medical").unwrap();
        assert!(!medical.enabled, "medical not in enabled set here");
        assert!(medical.term_count > 100, "bundled medical has a real list");
        let _ = std::fs::remove_dir_all(&dir);
    }

    // Custom words — add is persisted, reads back, and lands in the merged dict + jargon set when the pack
    // is enabled (the wiring that makes an added word actually get accepted). Also case-insensitive de-dup.
    #[test]
    fn custom_word_add_persists_and_feeds_the_dictionary() {
        let dir = temp_dir("custom-add");
        // A token guaranteed absent from curated so it can only appear via the custom pack.
        let term = "zzqcustomtoken";
        assert!(
            !FstDictionary::curated().contains_word_str(term),
            "premise: token must be absent from curated"
        );

        let words = add_custom_word(&dir, term).unwrap();
        assert_eq!(words, vec![term.to_string()]);
        assert_eq!(read_custom_words(&dir), vec![term.to_string()], "read back");

        // Case-insensitive de-dup: adding the upper-case variant must not create a second entry.
        let words2 = add_custom_word(&dir, "ZZQCUSTOMTOKEN").unwrap();
        assert_eq!(words2.len(), 1, "case-insensitive de-dup");

        // Enabling the `custom` pack must put the word in the dict AND the jargon (acceptance) set.
        let built = build(&dir, &[CUSTOM_PACK_ID.to_string()]);
        assert!(
            built.dict.contains_word_str(term),
            "custom pack on → word accepted by the dictionary"
        );
        assert!(
            built.jargon.contains(term),
            "custom word is in the acceptance-only jargon set"
        );

        // The label directive resolves to "My Words".
        let infos = pack_infos(&dir, &[CUSTOM_PACK_ID.to_string()]);
        let custom = infos.iter().find(|i| i.id == CUSTOM_PACK_ID).unwrap();
        assert_eq!(custom.label, "My Words");
        assert!(custom.enabled && custom.term_count == 1);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn custom_word_remove_takes_it_out_of_the_dictionary() {
        let dir = temp_dir("custom-remove");
        let term = "zzqremovable";
        add_custom_word(&dir, term).unwrap();
        assert!(build(&dir, &[CUSTOM_PACK_ID.to_string()])
            .dict
            .contains_word_str(term));

        let left = remove_custom_word(&dir, "ZZQREMOVABLE").unwrap(); // case-insensitive removal
        assert!(left.is_empty(), "word removed");
        assert!(
            !build(&dir, &[CUSTOM_PACK_ID.to_string()])
                .dict
                .contains_word_str(term),
            "removed word no longer accepted"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn custom_word_sanitize_rejects_bad_input() {
        assert!(sanitize_custom_word("   ").is_err(), "empty/blank rejected");
        assert!(
            sanitize_custom_word("two words").is_err(),
            "spaces rejected (Harper is per-token)"
        );
        assert!(
            sanitize_custom_word(&"x".repeat(61)).is_err(),
            "over-long rejected"
        );
        // Quote/angle characters are stripped, not fatal, leaving a clean token.
        assert_eq!(
            sanitize_custom_word("  meto\"prolol  ").unwrap(),
            "metoprolol"
        );
    }
}
