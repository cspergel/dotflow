# Medical Dictionary Pack — design

> Branch: `feat/review-enhancements`. Status: design (post-sweep, folded). Companion feature on the same
> branch: post-dictation review (separate design). Date: 2026-07-08.

> **Sweep fold (2026-07-08).** A 3-lens adversarial sweep (`/sweep`) HALTED this design with 4 unrefuted
> CRITICAL/HIGH findings; all are folded below and marked `[SWEEP-Fn]`. Headlines: (1) **feature was a
> no-op** — must parse the Document with the merged dict, not only the linter (CRITICAL); (2) **casing
> bypass** silently auto-applied medical terms — filter must be case-normalized (CRITICAL); (3) filter must
> guard the **original** span, not only the replacement (HIGH); (4) a test used a non-existent method and
> lacked feature-level teeth (HIGH). Verified-sound assumptions (do not re-litigate): `LintGroup::new_curated`
> genuinely uses the passed dict (`lint_group/mod.rs:856`); `MergedDictionary` unions lookups;
> `From<MutableDictionary> for FstDictionary` is cheap for ~5k terms; `DictWordMetadata::default()` passes the
> dialect gate; injecting pack vocab does **not** degrade curated grammar linting.

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
  `CachedDict` holds the `Arc<dyn Dictionary>` **and** the lowercase pack-term `HashSet` (used by the safety
  filter — built once alongside the FST). `set_enabled_packs(&[String])` (called at startup + on every
  settings change) rebuilds once; `current_dictionary() -> Arc<dyn Dictionary>` returns curated-only when
  nothing is enabled, else a `MergedDictionary(curated + pack FST)`.
- **[SWEEP-F7] Build outside the lock; degrade, never wedge.** `set_enabled_packs` builds the new
  `FstDictionary` into a **local**, then takes the lock only to swap the `Arc` — so a concurrent
  `current_dictionary()` never blocks for the FST build. If the build **panics or errors** (e.g. a malformed
  pack term), fall back to **curated-only + log a warning** rather than leaving the cache `None`/poisoned —
  because pack content is `include_str!`-fixed for the binary's life, a re-build-on-`None` would re-panic
  every call and silently disable *all* linting. `current_dictionary()` on the read path **only clones the
  cached `Arc`** — it never re-reads settings and never builds. Poison recovery (`.unwrap_or_else(|p|
  p.into_inner())`, as in `local_llm.rs:112`) is replicated verbatim at every lock site.
- Flow: settings change → command updates store + calls `set_enabled_packs` → cache rebuilds (build-then-swap)
  → `grammar::analyze` / `harper_cleanup` snapshot `current_dictionary()` **once** at the top and use it for
  **both** the linter (`LintGroup::new_curated`) **and** the Document parse (see [SWEEP-F1] in §2).

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

`noun_metadata()` = `DictWordMetadata::default()` (verified in the sweep: its empty `DialectFlags` passes the
dialect gate, so terms count as valid vocabulary; no POS metadata is needed for spell suppression).

**[SWEEP-F1] The load-bearing step — parse the Document with the merged dict.** Harper suppresses a Spelling
lint only when the word carries `Some(metadata)`, and that metadata is assigned **at parse time** by the dict
passed to `Document`. So swapping only the linter's dict is a **no-op**. Both `grammar::analyze` and
`harper_cleanup` must replace `Document::new_plain_english_curated(text)` with
`Document::new_plain_english(text, &dict)` using the *same* `current_dictionary()` value handed to
`LintGroup::new_curated`. (Confirmed by harper's own `issue_1876` test, which routes the merged dict to both.)

**[SWEEP-F3] Inflections.** `append_word_str` inserts only the exact surface form — no affix/plural expansion —
so `metoprolol` is accepted but `metoprolols` / `metoprolol's` are not. v1: include common inflected forms
directly in `medical.txt`; documented as a known limitation, not a silent gap.

Two consumers, two behaviors — **this is the safety design**:

- **`analyze` (Review panel, click-to-accept):** merged dict for both parse + lint. Terms accepted; any
  medical suggestion for a nearby typo is shown to the clinician, who chooses. Human-in-the-loop → safe.
- **`harper_cleanup` (silent auto-fix):** merged dict for acceptance, plus a **hardened drop filter** before
  applying any edit. The filter drops an edit when **either** side is medical:
  - **[SWEEP-F2, casing] Replacement side, case-normalized.** Drop if the replacement, **Unicode-lowercased**,
    is in the lowercase pack-term set. (Harper mirrors the offending word's casing into its suggestion, so a
    raw case-sensitive match misses `Metoprolol` at sentence start — the CRITICAL bypass.)
  - **[SWEEP-F2b] Original side.** Drop if the **original span text**, lowercased, is an accepted pack term —
    so a *valid* drug name can't be silently rewritten to a different word by a non-spell linter (the
    replacement-only guard could never catch that).
  - **[SWEEP-F4] All three `Suggestion` variants.** The filter is defined for `ReplaceWith` (check text),
    `InsertAfter` (check inserted text), and `Remove` (check the original span) — not only `ReplaceWith`.
  - Net: valid terms are never flagged, and no edit that inserts *or* overwrites a medical term is ever
    applied silently. The human-reviewed `analyze` path keeps surfacing suggestions for the clinician.

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

1. **Acceptance changes with the pack (both states) — the feature-level teeth.** Choose a term confirmed
   absent from curated Harper (`metoprolol`, `lisinopril`, `dyspnea` were verified absent in the sweep).
   **[SWEEP-F-t1] Guard the premise in-test:** assert `FstDictionary::curated().contains_word_str(term)
   == false` first, so a term drifting into curated (or dropped from the pack file) gives a clear diagnostic
   instead of a confusing failure. Then: with no packs `analyze` flags it Spelling; with medical on the flag
   is **gone**. This also covers [SWEEP-F1] — if the Document parse dict isn't swapped, the on-state still
   flags and this test fails loudly. Parameterize over 2–3 confirmed-absent terms.
2. **Safety filter — medical never auto-applied (boundary + fail case).** **[SWEEP-F2-test] Align the
   cross-check to the *primary* suggestion:** pick a typo for which Harper ranks the medical term **first**
   (since `harper_cleanup` only applies `suggestions.first()`), and assert in `analyze` that the medical term
   is the **first** replacement for that span. Then `harper_cleanup` with pack on leaves it **unchanged**
   (filter dropped it), while `analyze` **surfaces** it — proving the filter is path-specific, not a global
   no-op. **Add a casing case (the CRITICAL):** a **sentence-initial** typo whose first suggestion is the
   capitalized medical term (`Metoprolol`) must also be left unchanged by `harper_cleanup` — this fails if the
   filter isn't case-normalized. **Add the original-side case:** a valid pack term that a non-spell linter
   would rewrite must be left unchanged (guards [SWEEP-F2b]).
3. **No regression (fail case).** Pack on: `harper_cleanup("This is an test.")` → `"This is a test."`;
   clean prose untouched. Proves the merge didn't break curated linting.
4. **Registry drives the dict.** Enabled `["medical"]` → `current_dictionary().contains_word_str(term)`
   **[SWEEP-F3] `contains_word_str`, not `contains` — the latter doesn't exist on the trait.** Enabled `[]`
   → false. Narrow (wiring only) — it does **not** replace tests 1/2's feature-level teeth; keep all three.
5. **[SWEEP-F4] Pack-file parser boundaries.** Comment line (`# heading`) must NOT become vocabulary
   (assert a word from a comment is absent from `current_dictionary()`); trailing whitespace / blank lines
   yield clean entries; duplicate term doesn't panic the FST build; **empty pack file** builds without panic
   and behaves as pack-off; a term already in curated (`tachycardia`) doesn't double-flag or panic.

**Verification before done:** `cargo test --lib` (new + existing 188), then live — toggle the pack in the
running app and confirm a real clinical sentence stops underlining **and** that a sentence-initial drug-name
typo is not silently rewritten. Run `/verify` at the checkpoint so the referee carries the verdict.
