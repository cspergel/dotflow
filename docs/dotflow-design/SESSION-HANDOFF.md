# DotFlow — session handoff (resume here in a fresh session)

> Last updated end of the 2026-07-07 session. Everything below is on `origin/main` (HEAD `3d2fae1`), clean
> tree, CI (`code-quality` + `test`) green. Read this + [`ROADMAP.md`](./ROADMAP.md) to pick up.

## What DotFlow is

A fork of **Handy** (`cjpais/Handy`, MIT; Tauri 2 + Rust + React) rebranded to **DotFlow**. Local‑first,
**fully offline, privacy‑first** dictation + text tooling. Differentiators: **live in‑field dictation**
(Dragon feel), **dot‑phrase / voice‑alias macros**, a **typed text expander**, an **editable phrase
library**, **offline grammar/spelling cleanup + a Grammarly‑style review panel** (via Harper), and a
premium **Linear/Raycast‑style UI**.

- **Repo:** `github.com/cspergel/dotflow` (`origin`). `upstream` = `github.com/cjpais/Handy`.
- **Local path:** `~/Documents/Coding Projects/dotflow`. **Data dir:** `%APPDATA%/com.dotflow.app/`.
- **Design docs:** `docs/dotflow-design/` — `ROADMAP.md` (the plan), `DotFlow-plan-v2.md`, this file.
- **Fork maintenance:** `FORK.md` (upstream‑sync recipe, modified‑file list, §CI‑on‑the‑fork).

### Commercialization direction (discussed, not built)

Positioning: **private + offline + one‑time purchase** vs Grammarly/Dragon (cloud/subscription). Beachhead
vertical = **medical/clinical dictation** ("Dragon Medical, but private, offline, cheaper" — PHI never
leaves the device ⇒ no BAA/HIPAA‑cloud risk). Model: **open‑core** (free core + paid **Pro** one‑time license

- medical/legal **dictionary/template packs** + support). Licensing is clean: **MIT (Handy) + Apache‑2.0
  (Harper) + CC‑BY‑4.0 (Parakeet, commercial‑OK) all permit going proprietary** — just retain the notices in a
  licenses screen (no need to rewrite code to "escape" MIT). **Audit each bundled model's license before
  selling** (Whisper MIT ✓, Parakeet CC‑BY‑4.0 ✓; others TBD). The code isn't the moat — brand, curated domain
  packs, polish, compliance‑by‑architecture are.

## Environment / build / run / test

- Toolchain: cargo (MSVC), bun, node. `cargo` isn't on the Bash PATH by default → `export PATH="$HOME/.cargo/bin:$PATH"`.
- **No Vulkan SDK** → whisper `vulkan` feature dropped on x86_64‑windows; we run **Parakeet (GGUF via
  transcribe‑cpp, CPU)** — default model is `handy-computer/parakeet-tdt-0.6b-v3-gguf`.
- **Build:** `cd src-tauri && export CARGO_TARGET_DIR="C:/dtfb" && cargo build` (short dir dodges the Windows
  260‑char limit). `cargo build --bin dotflow` for just the app binary.
- **Run the built binary directly:** `"C:/dtfb/debug/dotflow.exe" --debug >/tmp/x.log 2>&1 &` — it connects
  to the Vite dev server (`:1420`) for the frontend and HMRs frontend edits. **Backend (Rust) changes need a
  rebuild + relaunch.**
- **Tests:** `cd src-tauri && cargo test --lib` (188 pass as of handoff). Frontend: `node_modules/.bin/tsc
--noEmit -p tsconfig.json`; `node_modules/.bin/eslint <files>`; `node_modules/.bin/prettier --write/--check`.
- **CI:** only `code-quality` + `test` run on the fork (the 7 heavy Handy workflows are disabled via
  `gh workflow disable` — see `FORK.md`). `format:check` runs prettier on the WHOLE repo incl. `.md`; the
  translation check requires **every locale** to have every key (add new keys to all 21 locales via a script).

## What WORKS today (all validated, on `origin/main`)

- **Dictation** (Parakeet CPU) + **live field streaming** + **dot‑phrase/voice‑alias macros** + **editable
  phrase library** (SQLite) + **instant macro insert**. (From the prior session; still good.)
- **Typed text expander** (`experimental_typed_expander`, opt‑in): a global Windows Raw Input keyboard monitor
  expands your dot‑triggers (`.fu`) in ANY app. `dotflow/typed_expander/backend.rs`. Self‑suppresses via the
  re‑entrant `clipboard::injection_guard`. Toggle + ding‑toggle now live in the **Phrases** section.
- **Premium redesign:** surface‑ladder tokens (off‑white canvas → white panels → inset controls, green‑biased
  hairlines, **no shadows**), **Geist** bundled font, emerald reserved for meaning, grouped **DotFlow‑specific
  sidebar** (DICTATE / REVIEW / SYSTEM, Phrases elevated), **super‑compact "mini" bar** tier, trimmed model
  catalog (8 curated models). Donate + acknowledgements removed. "Handy Keys" → "DotFlow Keys".
- **Clean‑up‑selected‑text hotkey** (`Ctrl+Shift+U`, rebindable): copies the selection → cleans it → pastes
  back → restores clipboard. **Tiered engine:** post‑process LLM (if configured) → **Harper (offline grammar/
  spelling)** → deterministic mechanical tidy. Reliable now (waits for modifier release before the synthetic
  Ctrl+C; sentinel‑based copy detection).
- **Cleanup settings section** (sidebar → Cleanup): rebindable hotkey, engine indicator (Offline/AI),
  **"Try it"** box with **Auto‑fix** and an interactive **Review** panel.
- **Review panel** (`ReviewPanel.tsx`): Grammarly‑style — text with issues underlined, click‑to‑accept, per‑
  issue cards with each replacement + Ignore, Accept‑all, Copy result. Offline via Harper's `analyze_text`
  (returns char‑span + kind + message + replacements). Stable‑offset model (analyze once; accept/ignore are
  flags; result computed by splicing) so offsets never drift.

## Hard‑won gotchas (do NOT re‑derive)

- **Hotkey‑triggered synthetic keys are polluted by the still‑held trigger modifiers.** `Ctrl+Shift+U` → the
  user is holding Ctrl+Shift when we send Ctrl+C → OS sees Ctrl+Shift+C → copies nothing ("works 1 in 10").
  **Fix:** `input::wait_for_modifiers_released()` (polls `GetAsyncKeyState`) before the copy, + `release_modifiers`.
- **Ctrl+Alt+\* is AltGr on Windows** and gets swallowed by the layout — never use it for a global hotkey.
- **A panic in a `tauri::async_runtime::spawn` task vanishes silently** (no log) — the action just "does
  nothing." Harper's `harper_cleanup`/`analyze` are wrapped in `catch_unwind`. When debugging "nothing
  happens," add an INFO log at the fire point to disambiguate "didn't fire" vs "fired then panicked".
- **Shortcut validator now rejects modifier‑less global bindings** (a bare `l` fired on every keypress and
  trapped the user). Escape + F‑keys exempt. `shortcut/tauri_impl.rs`.
- **Single‑instance forwarding bites during testing:** launching `dotflow.exe` while another instance runs
  forwards args and exits (new binary doesn't run). Always `taskkill //F //IM dotflow.exe` and confirm none
  running before relaunching a rebuilt binary. `cargo test` can hit "os error 32" (running app locks a staged
  DLL) → kill the app first.
- **specta bindings** (`src/bindings.ts`) only regenerate when the app actually RUNS (debug export in
  `run()`), not on `cargo build`/`cargo test`. When you add a command, add its binding (and any Type) to
  `bindings.ts` by hand to use it immediately; the app will normalize it on next launch.
- **Harper API** (`harper-core` 2.5.0): `Document::new_plain_english_curated(text)` → `LintGroup::new_curated(
FstDictionary::curated(), Dialect::American).lint(&doc)` → `Vec<Lint{ span: Span<char>, lint_kind, message,
suggestions }>`. `Suggestion::apply(span, &mut Vec<char>)`. Spans are CHAR offsets. Skip `LintKind::Enhancement`
  (subjective). Apply edits back‑to‑front / non‑overlapping. `wgpu` in `cargo add`'s output was a phantom
  (not in the normal build graph) — Harper builds clean, ~2.5 min first time.
- **Windows 11 Notepad has built‑in autocorrect** — a user reported "instant corrections while typing"; it
  was Notepad, not DotFlow. DotFlow has NO type‑time correction (expander only does dot‑triggers).
- The **empty‑key `.` trigger bug** (fixed): alias‑only phrases have `key=""` → `.{key}`==`.` matched every
  period. `PhraseTable::new` drops empty keys.

## ✅ DONE on branch `feat/selection-review-overlay` (2026-07-08 — not yet merged to main)

The selection→review overlay AND a local-AI backend are built, code-reviewed, and verified live on Win11
(25 commits, all builds green, 188 tests). See `docs/plans/2026-07-07-selection-review-overlay-{design,implementation}.md`.

- **Selection → review overlay:** hotkey `Ctrl+Shift+J` grabs the selection → a **draggable, content-sized,
  click-away-dismiss** card near the cursor → offline **Proofread** (Harper, via `ReviewPanel`) + **AI chips**
  (Rewrite/Formal/Summarize) → Apply pastes back (single Ctrl+Z reverts). Key files: `overlay.rs`
  (`create/show/hide_review_overlay`), `actions.rs` (`ReviewSelectionAction`, `copy_selection`),
  `commands/cleanup.rs` (`apply_review_result`/`cancel_review`/`get_pending_review`),
  `src/overlay/review/ReviewOverlay.tsx`, `capabilities/default.json` (the window must be listed there).
- **Local offline AI (`local-llm` cargo feature, default OFF):** `llama-cpp-2` runs GGUF models on CPU
  (`dotflow/local_llm.rs`, model cached). `ai_transform` routes cloud/Ollama → local → error. **Model picker**
  in Settings→Cleanup (`dotflow/llm_catalog.rs`, `commands/llm.rs`, `LlmModelPicker.tsx`): download/select/
  cancel/delete; **Gemma 4 E2B (Apache-2.0) recommended default**. Models in `%APPDATA%/…/models/llm/`.
  Feature build needs libclang + cmake (see the gotchas below / cross-session memory).
- **Nemotron ASR streaming** surfaced in the STT picker (allowlist add in `managers/model.rs`).

**Left:** live acceptance test (F1 clipboard-preservation + AI round-trip), decide merge-to-main, macOS/Linux
local-llm (Win-x86_64 only), a few Minor polish items.

### Gotchas added this branch
- **Phantom-Alt:** `input::release_modifiers` only releases modifiers actually held (GetAsyncKeyState) — a
  spurious Alt-up flips WinUI apps into Alt-menu mode and eats the synthetic Ctrl+C.
- **Feature build env:** `export LIBCLANG_PATH="C:/Users/drcra/anaconda3/Lib/site-packages/clang/native"` +
  VS 2022 CMake on PATH; `cargo build --bin dotflow --features local-llm`.
- **Tauri v2 capabilities:** a new window must be in `capabilities/default.json` `windows[]` or its frontend
  window calls (startDragging, setSize) are silently denied.

---

## (superseded) original sketch — selection hotkey → floating review overlay

**Goal (agreed):** a 2nd hotkey grabs the current selection (or whole field) → a small **always‑on‑top
DotFlow window pops near the cursor** showing the **review panel** → user accepts fixes → pastes back. This is
the "edit where I am" answer (a floating card, NOT literal in‑app editing — that's deferred). Model the UX on
**WritingTools** (studied, GPL — don't copy code): an **action menu** (Proofread=Harper offline / Rewrite /
Formal / Summarize=LLM tiers), cursor‑anchored popup, **single Ctrl+Z reverts** the paste.

Build notes (the review panel + engine already exist — this is the window + plumbing):

- New binding `review_selection` (give it a modifier combo; **not** Ctrl+Alt+\*). Action mirrors
  `cleanup_selection`'s copy phase (wait‑for‑release + sentinel copy) but instead of auto‑pasting, it opens
  the overlay window with the selected text.
- **Overlay window:** a small always‑on‑top, frameless Tauri window (there's precedent — the recording
  `overlay/` window + the frameless main window). Route the selected text to it (Tauri event/command), render
  `ReviewPanel`, and on "Apply" paste the result via `clipboard::inject_bulk` (reuse the guard + restore).
- **Also wire post‑dictation review:** after a dictation finalizes, optionally pop the same overlay with the
  transcript for review before insertion (setting‑gated).
- Keep it isolated so the same panel serves: Try‑it box (done), selection overlay, post‑dictation.

## Roadmap / tiering / future (see ROADMAP.md §5 for detail)

The cleanup engine is a **ladder**: (1) deterministic mechanical ✅ → (2) **Harper** rule‑based offline ✅ →
(3) **ML GEC** (export CoEdIT/GECToR to ONNX, run on the `ort` runtime we already ship — the "post‑ML" tier,
catches contextual errors; Gramformer is the concept ref) ⏳ → (4) **LLM** (local Ollama / cloud) ✅.

Backlog, roughly ordered: selection overlay + post‑dictation review (**next**) · **medical/legal dictionary
packs** (toggleable; `hunspell-en-med-glut` ~90k terms, small — bundle or download‑on‑demand) · **ML GEC
tier** · **terminal mode** (tuipo is the blueprint — Rust, MIT/Apache, pseudo‑terminal + **Harper** underlines,
skips code patterns; on‑demand review good, live auto‑correct in terminals = bad) · **Chrome extension**
(deferred; ai‑grammar is the ref — MIT, Chrome built‑in AI / Ollama) · **phone‑as‑microphone** · Mac support
(the expander/raw‑input is Windows‑only) · weekly upstream sync (last: `0a59e1f`).

**Repos reviewed this session (inspiration; note licenses):** WritingTools (GPL — study only, UX north star)
· ClipSlop (MIT — prompt chaining idea) · tuipo (MIT/Apache — terminal + Harper blueprint) · Gramformer
(MIT — ML GEC concept) · ai‑grammar (MIT — Chrome ext ref). GrammarFixer (basic, skip).

## Key files

- **Grammar/cleanup:** `src-tauri/src/dotflow/grammar.rs` (Harper: `harper_cleanup` + `analyze` + `TextSuggestion`,
  panic‑safe) · `dotflow/cleanup.rs` (deterministic, tested) · `actions.rs` (`resolve_cleanup`, the cleanup
  hotkey action, `wait_for_modifiers_released`/sentinel copy) · `commands/cleanup.rs` (`preview_cleanup`,
  `analyze_text`, `post_process_is_configured`) · `input.rs` (`send_copy_ctrl_c`, `release_modifiers`,
  `wait_for_modifiers_released`).
- **Cleanup UI:** `src/components/settings/cleanup/CleanupSettings.tsx` + `ReviewPanel.tsx`.
- **Typed expander:** `src-tauri/src/dotflow/typed_expander/{mod.rs,backend.rs}`; toggles now in
  `settings/phrases/PhrasesSettings.tsx` (`TypedExpander.tsx`, `TypedExpanderSound.tsx`).
- **Shortcuts:** `src-tauri/src/shortcut/{tauri_impl.rs (validate + register),handler.rs}`; bindings +
  `ACTION_MAP` in `actions.rs`; defaults in `settings.rs` (`get_default_settings`).
- **Injection:** `clipboard.rs` (`is_injecting`/`injection_guard` re‑entrant counter, `inject_bulk`,
  `inject_field_edit`). **Redesign tokens:** `src/styles/theme.css` + `src/App.css` (`@theme`, Geist `@font-face`).
- **Sidebar/IA:** `src/components/Sidebar.tsx` (`SECTIONS_CONFIG` + groups). **View modes:** `src/App.tsx`
  (full / bar / mini). **Model catalog trim:** `src-tauri/src/managers/model.rs` (`DOTFLOW_MODEL_REPOS`).
- **Fonts:** `src/assets/fonts/Geist-Variable.woff2` (+ `NOTICE.md`).
