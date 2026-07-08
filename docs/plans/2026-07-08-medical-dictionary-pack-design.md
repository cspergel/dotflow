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
2. **Packaging — RUNTIME-LOADABLE (revised 2026-07-08).** Packs are **external `.txt` files** discovered at
   runtime in `%APPDATA%/com.dotflow.app/dictionaries/`. The default **medical** pack ships as a bundled Tauri
   **resource** and is **seeded** into that dir on first run (only if absent — never overwrites user edits).
   Users can drop in / (later) download additional packs with **no rebuild** — matching the sell-packs-as-
   add-ons commercial model. (Supersedes the earlier `include_str!`-bundled-only decision.)
3. **v1 scope:** **acceptance-only (safe).** Valid terms stop being flagged; medical terms are **never
   silently auto-applied** as corrections (filtered out of the `harper_cleanup` auto-fix path). They may
   still surface in the human-reviewed Review panel. Eliminates any wrong-drug silent-correction risk.
4. **Architecture:** a **generic pack registry** built by **scanning the dictionaries dir** — one `.txt` file
   = one pack (id = filename stem). Adding legal later = drop a `legal.txt` in the folder, no code change.
5. **Toggle location:** Settings → Cleanup (where Harper engine settings already live; packs affect every
   Harper path). Include an "Open dictionaries folder" affordance and a "Reload packs" action.

> **Runtime-loadable delta.** External files are **untrusted input** (user-editable, possibly malformed or
> huge). New robustness obligations, folded into §1/§2/§4 and marked `[LOAD-Fn]`: per-file parse isolation (a
> bad file skips itself, never crashes the app or breaks other packs), a size/term cap (no OOM), UTF-8-lossy
> read, and the [SWEEP-F7] degrade-to-curated guarantee now applies **per pack**. Content can change between
> runs, so the cache also reloads on explicit user action (toggle / Reload), not only at startup — a bad
> hand-edit is recoverable without a reinstall. This new loading/seeding surface should get a focused
> re-sweep before implementation.

## Section 1 — Data model & flow

New module `src-tauri/src/dotflow/dictionary_packs.rs`:

```rust
pub struct DictionaryPack { pub id: String, pub label: String, pub path: PathBuf }

/// Seed bundled defaults (only if absent), then discover every *.txt in the dictionaries dir.
pub fn discover_packs(dir: &Path) -> Vec<DictionaryPack>;   // id = filename stem, label = titlecased id
fn seed_defaults(dir: &Path);                               // copy resources/dictionaries/medical.txt if missing
```

- **Dictionaries dir:** `%APPDATA%/com.dotflow.app/dictionaries/` (resolve via Tauri path API, not a hardcoded
  string). Created on first run.
- **Seeding [LOAD-seed]:** the default `medical.txt` ships as a Tauri **resource** (`resources/dictionaries/
  medical.txt`, listed in `tauri.conf.json` `bundle.resources`). On startup, `seed_defaults` copies it into
  the dir **only if a file of that name does not already exist** — so a user who edits/curates their medical
  pack is never clobbered by an update.
- **Discovery:** scan the dir for `*.txt` (ignore other files, dotfiles, subdirs). Each file → a pack; `id` =
  lowercased filename stem (`medical.txt` → `medical`); `label` = titlecased id. Duplicate/ill-formed names
  are de-duped by id (first wins) + logged.
- **File format:** one term per line; `# …` comment lines and blank lines skipped; an optional first-line
  `# label: Medical Terminology` overrides the derived label. UTF-8, read **lossy** [LOAD-F-enc].
- **[LOAD-F-iso] Per-file isolation + caps:** each file is read/parsed independently inside a `Result`/
  `catch_unwind`; a file that is unreadable, invalid, exceeds a **size cap** (e.g. 8 MB) or **term cap**
  (e.g. 500k) is **skipped with a warning** and does **not** disable other packs or crash the app.
- Settings (`settings.rs`): add `enabled_dictionary_packs: Vec<String>`, default `[]` (opt-in, OFF). Enabled
  ids that no longer correspond to a discovered file are simply ignored (stale-toggle tolerant).
- Process-wide cached dictionary: `Mutex<Option<CachedDict>>` keyed on the sorted enabled-pack-id set.
  `CachedDict` holds the `Arc<dyn Dictionary>` **and** the lowercase pack-term `HashSet` (used by the safety
  filter — built once alongside the FST). `set_enabled_packs(&[String])` (called at startup + on every
  settings change) rebuilds once; `current_dictionary() -> Arc<dyn Dictionary>` returns curated-only when
  nothing is enabled, else a `MergedDictionary(curated + pack FST)`.
- **[SWEEP-F7 + LOAD-F-iso] Build outside the lock; degrade per pack, never wedge.** `set_enabled_packs`
  builds the merged `FstDictionary` into a **local**, then takes the lock only to swap the `Arc` — so a
  concurrent `current_dictionary()` never blocks for the FST build. Each enabled pack is folded in
  independently: a pack whose file fails to read/parse/build (panic or error) is **skipped with a warning**
  and the merge continues with curated + the packs that did build. The result is **never** left
  `None`/poisoned — worst case is curated-only. `current_dictionary()` on the read path **only clones the
  cached `Arc`** — it never re-reads settings, re-scans the dir, or builds. Poison recovery
  (`.unwrap_or_else(|p| p.into_inner())`, as in `local_llm.rs:112`) is replicated at every lock site.
- **Reload semantics:** the cache is rebuilt at startup, on a pack **toggle**, and on an explicit **Reload
  packs** action (which also re-runs discovery). Editing a `.txt` on disk takes effect on the next
  reload/restart — no live file-watcher in v1 (documented, not silent).
- Flow: settings change / reload → command re-discovers packs + calls `set_enabled_packs` → cache rebuilds
  (build-then-swap) → `grammar::analyze` / `harper_cleanup` snapshot `current_dictionary()` **once** at the
  top and use it for **both** the linter (`LintGroup::new_curated`) **and** the Document parse ([SWEEP-F1]).

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

Commands (`commands/dictionary.rs`):

```rust
get_dictionary_packs(app) -> Vec<DictionaryPackInfo>   // {id, label, enabled, term_count}, from dir scan
set_dictionary_pack_enabled(app, id: String, enabled: bool) -> Result<(), String>
reload_dictionary_packs(app) -> Vec<DictionaryPackInfo> // re-scan dir + rebuild cache (picks up dropped/edited files)
open_dictionaries_folder(app) -> Result<(), String>     // reveal %APPDATA%/…/dictionaries in the file manager
```

- `get_dictionary_packs` runs `seed_defaults` (first-run) then `discover_packs`, cross-referencing the
  settings enabled set. `set_dictionary_pack_enabled` writes the store then calls `set_enabled_packs` → live
  rebuild, no restart. `reload_dictionary_packs` re-discovers (for a file the user just dropped in / edited).
- Startup (`lib.rs`): seed defaults, then call `set_enabled_packs` once from persisted settings before first
  cleanup.
- Hand-add commands + `DictionaryPackInfo` to `src/bindings.ts` (bindings only regenerate on app run).
- UI: new "Dictionaries" subsection in `CleanupSettings.tsx` — one toggle row per discovered pack (label +
  term count), an **Open dictionaries folder** button and a **Reload** button, plus a one-line hint that
  users can drop their own `.txt` term lists in the folder. i18n keys → `en/translation.json` + propagated to
  all 21 locales (translation-completeness CI check).
- **Out of scope (YAGNI):** per-term in-app editing, pack download/marketplace UI, license-gating, legal pack
  content, live file-watching (Reload button covers it).

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
6. **[LOAD] Discovery + seeding (both states).** Given a temp dir: `discover_packs` finds a `medical.txt` as
   pack id `medical`, ignores non-`.txt` files/dotfiles/subdirs. `seed_defaults` **creates** `medical.txt`
   when absent, and **does NOT overwrite** an existing file whose contents differ (write a sentinel term,
   seed, assert the sentinel survives) — the never-clobber-user-edits guarantee.
7. **[LOAD-F-iso] Bad-file isolation (teeth).** A dir with a good `medical.txt` and a **malformed** pack
   (e.g. a huge/over-cap file, or invalid bytes): building the merged dict **skips the bad file with a
   warning**, still accepts terms from `medical.txt`, and never panics or returns curated-only-because-of-the-
   bad-one. Removing the isolation (letting the bad file into the build) must make this test fail — that's the
   canary that the degrade-per-pack guard has teeth.
8. **[LOAD] Reload picks up a newly dropped file.** After `set_enabled_packs(["legal"])` with no `legal.txt`
   present (no-op, tolerated), drop a `legal.txt` with a term, call the reload path, enable it → the term is
   now accepted. Proves discovery is re-run, not cached from startup.

**Verification before done:** `cargo test --lib` (new + existing 188), then live — toggle the pack in the
running app and confirm a real clinical sentence stops underlining **and** that a sentence-initial drug-name
typo is not silently rewritten. Run `/verify` at the checkpoint so the referee carries the verdict.
