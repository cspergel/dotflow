# Medical Dictionary Pack — design

> Branch: `feat/review-enhancements`. Status: design (pre-implementation). Companion feature on the same
> branch: post-dictation review (separate design). Date: 2026-07-08.

## Goal

A toggleable, **fully-offline** clinical-terms dictionary that extends Harper so valid medical terms (drug
names, standard abbreviations, anatomy, specialty terms) are **not flagged as misspellings** in DotFlow's
cleanup / review / selection-overlay flows. Feeds the future paid medical Pro pack, so the term list's
license must be clean.

## Decisions (locked via brainstorming)

1. **Term source:** curated **in-house** list we own outright (~2–5k high-value terms). Zero license risk,
   ships fast, grows over time. (Rejected: `hunspell-en-med-glut` — license likely copyleft, risky under a
   proprietary SKU; public-domain NLM/RxNorm — more assembly, deferred as a later scale-up source.)
2. **Packaging:** **bundle** the list into the binary (`include_str!`). It's tiny; no download-on-demand.
3. **v1 scope:** **acceptance-only (safe).** Valid terms stop being flagged; medical terms are **never
   silently auto-applied** as corrections (filtered out of the `harper_cleanup` auto-fix path). They may
   still surface in the human-reviewed Review panel. Eliminates any wrong-drug silent-correction risk.
4. **Architecture:** a **generic pack registry** (`{id, label, terms}`), shipping **only** the medical pack
   now. Adding legal later = one term file + one registry entry, no schema change.
5. **Toggle location:** Settings → Cleanup (where Harper engine settings already live; packs affect every
   Harper path).

## Section 1 — Data model & flow

New module `src-tauri/src/dotflow/dictionary_packs.rs`:

```rust
pub struct DictionaryPack { pub id: &'static str, pub label: &'static str, pub terms: &'static str }
pub static PACKS: &[DictionaryPack] = &[DictionaryPack {
    id: "medical", label: "Medical",
    terms: include_str!("../../resources/dictionaries/medical.txt"),
}];
```

- Term list: `src-tauri/resources/dictionaries/medical.txt`, one term per line, `#` comments allowed,
  compiled in via `include_str!`. No runtime file, no network.
- Settings (`settings.rs`): add `enabled_dictionary_packs: Vec<String>`, default `[]` (opt-in, OFF).
- Process-wide cached dictionary: `Mutex<Option<CachedDict>>` keyed on the sorted enabled-pack-id set.
  `set_enabled_packs(&[String])` (called at startup + on every settings change) rebuilds once;
  `current_dictionary() -> Arc<dyn Dictionary>` returns curated-only when nothing is enabled, else a
  `MergedDictionary(curated + pack FST)`. Mirrors `local_llm.rs`'s `MODEL_CACHE`.
- Flow: settings change → command updates store + calls `set_enabled_packs` → cache rebuilds →
  `grammar::analyze` / `harper_cleanup` call `current_dictionary()` instead of `FstDictionary::curated()`.

## Section 2 — Harper integration + acceptance-only safety filter

Build merged dict from enabled packs:

```rust
let mut md = MutableDictionary::new();
for pack in enabled_packs {
    for line in pack.terms.lines() {
        let w = line.trim();
        if !w.is_empty() && !w.starts_with('#') { md.append_word_str(w, noun_metadata()); }
    }
}
let pack_dict: Arc<FstDictionary> = Arc::new(md.into());   // From<MutableDictionary>
let mut merged = MergedDictionary::new();
merged.add_dictionary(FstDictionary::curated());
merged.add_dictionary(pack_dict);
```

`noun_metadata()` = permissive noun-ish `DictWordMetadata` so terms count as real vocabulary. (Exact
constructor confirmed against extracted harper-core 2.5.0 source at implementation time.)

Two consumers, two behaviors — **this is the safety design**:

- **`analyze` (Review panel, click-to-accept):** `current_dictionary()` as-is. Terms accepted; any medical
  suggestion for a nearby typo is shown to the clinician, who chooses. Human-in-the-loop → safe to surface.
- **`harper_cleanup` (silent auto-fix):** `current_dictionary()` for acceptance, **but drop any edit whose
  replacement is a medical-pack term** before applying. A `HashSet<String>` of enabled-pack terms (built
  with the FST) is the filter. Net: valid terms never flagged *and* no medical term silently inserted.

## Section 3 — Commands, bindings & UI

Commands (`commands/dictionary.rs` or fold into `cleanup.rs`):

```rust
get_dictionary_packs(app) -> Vec<DictionaryPackInfo>   // {id, label, enabled}
set_dictionary_pack_enabled(app, id: String, enabled: bool) -> Result<(), String>
```

- `set_dictionary_pack_enabled` writes the store then calls `set_enabled_packs` → live rebuild, no restart.
- Startup (`lib.rs`): call `set_enabled_packs` once from persisted settings before first cleanup.
- Hand-add commands + `DictionaryPackInfo` to `src/bindings.ts` (bindings only regenerate on app run).
- UI: new "Dictionaries" subsection in `CleanupSettings.tsx`, one toggle row per pack from
  `get_dictionary_packs`, short description. i18n keys → `en/translation.json` + propagated to all 21
  locales (translation-completeness CI check).
- **Out of scope (YAGNI):** per-term user editing, custom user dictionary UI, legal pack content.

## Section 4 — Testing (charter: tests must have teeth)

Each test asserts Harper's **real output**, checks **both states** (pack on/off), plus the safety boundary
and a must-not-regress case.

1. **Acceptance changes with the pack (both states).** A term in our list but NOT in curated Harper (e.g.
   `metoprolol`): with no packs `analyze` flags it Spelling; with medical on the flag is gone. Identical
   on/off output ⇒ merge did nothing ⇒ test fails.
2. **Safety filter — medical never auto-applied (boundary + fail case).** A typo whose edit-distance
   neighbor is a medical term: `harper_cleanup` with pack on leaves it **unchanged**; the same input in
   `analyze` **does** surface the suggestion (proves the filter is path-specific, not a global no-op).
3. **No regression (fail case).** Pack on: `harper_cleanup("This is an test.")` → `"This is a test."`;
   clean prose untouched. Proves the merge didn't break curated linting.
4. **Registry drives the dict.** Enabled `["medical"]` → `current_dictionary().contains(term)` true;
   enabled `[]` → false. Asserts on real `contains`, not a constructed value.

**Verification before done:** `cargo test --lib` (new + existing 188), then live — toggle the pack in the
running app and confirm a real clinical sentence stops underlining. Run `/verify` at the checkpoint so the
referee carries the verdict.
