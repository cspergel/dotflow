# DotFlow — session handoff (resume here in a fresh session)

## What DotFlow is

A fork of **Handy** (`cjpais/Handy`, MIT; Tauri 2 + Rust + React) rebranded to **DotFlow**. Local‑first
dictation app whose differentiators are: **live in‑field text injection as you speak** (Dragon feel),
**dot‑phrase / voice‑alias macros**, an **editable phrase library**, and a **Dragon‑style compact UI**.

- **Repo:** `github.com/cspergel/dotflow` (`origin`). `upstream` = `github.com/cjpais/Handy`.
- **Local path:** `~/Documents/Coding Projects/dotflow`.
- **Product design:** `docs/dotflow-design/DotFlow-plan-v2.md` + `DotFlow-asr-stack-research.md`.
- **Fork maintenance:** `FORK.md` (upstream‑sync recipe + the exact list of Handy files we modified).

## Environment / build / run

- Toolchain: cargo (MSVC on Windows), bun, node. **No Vulkan SDK** → whisper `vulkan` feature dropped in
  `src-tauri/Cargo.toml` (we use Parakeet ONNX/CPU).
- **Build:** `cd src-tauri && export CARGO_TARGET_DIR="C:/dtfb" && cargo build` (short target dir dodges the
  Windows 260‑char path limit).
- **Run (dev):** `cd dotflow && export CARGO_TARGET_DIR="C:/dtfb" && bun run tauri dev` — the watcher
  auto‑rebuilds Rust on save and HMRs the frontend. A **frameless** window / **icon** change needs a full
  app restart (and icon needs `touch src-tauri/build.rs` to re‑embed).
- **Tests:** `cd src-tauri && cargo test --lib dotflow` (the pure cores are unit‑tested).
- **Frontend typecheck:** `node_modules/.bin/tsc --noEmit -p tsconfig.json`.
- **Data dir:** `%APPDATA%/com.dotflow.app/` — `phrases.db`, `settings_store.json`, the Parakeet model.
- **CI:** `code-quality` + `test` PASS. The heavy Handy workflows (`build`, `main-build`, `release`,
  `nix-check`, `playwright`, `build-test`, `pr-test-build`) **fail on the fork** (need Handy's secrets/signing)
  — recommended to disable them; **not done yet** (open task).

## What WORKS today (validated live)

- **Live field streaming** (Dragon feel): a streaming model's committed text is keystroke‑injected as you
  speak. Char‑by‑char with a **tunable per‑char delay** (`field_stream_char_delay_ms`, default 8) + a
  **throttle** (`field_stream_throttle_ms`, default 100) — this beats the Windows enigo key‑repeat race.
  Whole‑word hold + command‑buffer. Gated by `experimental_field_streaming` (needs a streaming model, e.g.
  Parakeet Unified). Sliders in Advanced → Experimental.
- **Dot‑phrase / voice‑alias macros:** spoken triggers (`insert follow up`) or dot keys (`.fu`) expand to a
  saved block. Matching is **case‑, punctuation‑, and hyphen‑insensitive** (Parakeet writes "follow‑up").
- **Instant macro insert:** the finalize path pastes the resolved block via **clipboard (`inject_bulk`)** so a
  macro drops in at once on release (streaming mid‑utterance still types word‑by‑word — accepted).
- **Editable phrase library:** SQLite `PhraseManager` + CRUD commands + a **Phrases** settings page; edits
  apply on the next dictation.
- **Dragon‑style UI:** frameless window; custom **titlebar** (drag / minimize / close‑to‑tray); a compact
  **always‑on‑top bar** (mic **amber on standby → green while dictating**, "Ready to dictate", hotkey on
  hover, drag anywhere) that **expands** to the full app. Emerald theme, DotFlow wordmark + dot‑flow mark,
  green app icons.

## Hard‑won gotchas (do NOT re‑derive)

- `enigo.text()` **races on this Windows machine** (dropped/repeated keys, "ggggg") — the race is INSIDE a
  single multi‑char call, so throttling whole calls can't fix it. **Fix = type char‑by‑char with a per‑char
  delay** (`inject_field_edit` in `clipboard.rs`).
- **Parakeet hyphenates** "follow up" → "follow‑up" → matching uses `canonical_words` (split on hyphens) in
  `dotflow/phrases.rs`.
- **Tauri v2 gates window ops** (`setSize`, `startDragging`, …) behind **capabilities** → see
  `src-tauri/capabilities/default.json`. Missing perms fail silently.
- **Frameless** = `decorations(false)` in `lib.rs` + custom chrome; the app **icon** is embedded at build via
  `build.rs`, so changing `icons/` needs `touch src-tauri/build.rs` to force a re‑embed, then a full restart.
- **CI:** `format:check` runs `prettier --check .` (WHOLE repo incl. `.md`) **&&** `cargo fmt -- --check`; the
  translation check requires **every locale** to have every key (add new keys to all 20 non‑EN locales).
  `code-quality` only triggers on `src/**` changes — dispatch it manually if you only changed Rust/docs.

## This session's work (all on `origin/main`)

Phrase pipeline (injection race fix → hyphen matching → editable library) · full rebrand (emerald theme,
wordmark/mark, green icons, "Handy"→"DotFlow", title) · Dragon‑style shell (frameless + custom titlebar +
compact bar + window resize + `dictation-state` color signal) · instant macro insert (`inject_bulk`) ·
DotFlow README + `FORK.md` · CI fixes (translations/eslint/prettier/cargo fmt green) · ported 2 upstream Handy
fixes (`0a59e1f` ampersands, `cdb4633` overlay) · **typed‑expander step 1**.

## IN PROGRESS — Typed text expander (Beeftext/Espanso‑style)

Goal: type a dot‑trigger (`.fu`) in **any** app and it's replaced by your saved text — the **same phrase
library** that powers spoken triggers. **Architecture decision (from research):** build a global‑input
expander **modeled on Espanso** (Espanso is Rust but **GPL‑3.0 → study/mirror, do NOT link its crates**;
Beeftext is **MIT**). Reject the IME approach for v1 (Windows TSF is heavy + forces input‑source switching).

**Step 1 — DONE (commit `8c4eb61`), off by default, dictation untouched:**

- Setting `experimental_typed_expander` (default OFF).
- **Self‑injection suppression flag** in `clipboard.rs`: `is_injecting()` + an `InjectGuard` raised around
  every injection (`inject_field_edit`, `inject_bulk`, `inject_text_raw`, `paste`). The future keyboard
  monitor checks this and drops all input while raised → DotFlow can never re‑trigger itself. No‑op today.
- **Pure, tested core:** `dotflow::typed_expander::ExpanderBuffer` (rolling buffer + push/backspace/reset/
  consume) and `PhraseTable::match_typed_trigger` (dot‑trigger suffix match, case‑insensitive, longest‑key‑
  wins). 8 unit tests.

**NEXT — Step 2/3: the Windows Raw Input backend (the actual keyboard monitor).** Build spec (from research,
so you don't have to re‑research):

- **Detect via Raw Input, NOT `WH_KEYBOARD_LL`** (a slow LL hook lags the whole system + Windows can drop it).
  Use the `windows` crate: `RegisterRawInputDevices` (keyboard, `RIDEV_INPUTSINK`) + a **message‑only hidden
  window** + `WM_INPUT` pumped with `GetMessage` on a **dedicated native thread** (not the Tauri thread).
  Decode characters with `ToUnicodeEx` (full keyboard state) — Raw Input gives keys, you reconstruct chars.
- **Feed** printable chars → `ExpanderBuffer::push`; Backspace → `backspace`; Enter/Tab/Esc + arrow/nav keys +
  mouse click + **window‑focus change** (`SetWinEventHook EVENT_SYSTEM_FOREGROUND`) → `reset`.
- **On `matched()`:** if `!clipboard::is_injecting()`, raise the guard, **`SendInput` N× `VK_BACK`** to erase
  the `.key`, then paste the expansion via **`inject_bulk`** (instant, reuses what's built), then
  `buffer.consume(N)`. Keep the buffer's suppression tight (a small trailing settle after emit so async
  `WM_INPUT` for our own keys doesn't leak — or filter by HID source like Espanso).
- **Wire‑up:** start the monitor thread only when `experimental_typed_expander` is on (start on setting‑on,
  stop on setting‑off); a UI toggle in Advanced → Experimental with an explicit "monitors your typing" note.
- **Keep it isolated** in `dotflow/typed_expander/` behind a trait so mac/Linux backends can follow later.

## Roadmap / backlog (agreed)

1. **Typed expander backend** (next — spec above).
2. **Premium redesign** of the full/expanded window — the Linear/Raycast look: surface‑ladder colors + 1px
   hairline borders + **no drop shadows** + **medium‑weight (500) headings, tight tracking** + constrained
   content width (~640–720px) + settings as **grouped hairline‑separated rows** (not card‑per‑setting). (Full
   research spec was captured in‑session; re‑fetch if needed.)
3. **"Clean up selected text" hotkey** — a 2nd hotkey that sends the SELECTED text (typed or not) to the
   **post‑process LLM** (Ctrl+C → read clipboard → LLM cleanup prompt → paste result). Reuses existing
   post‑processing infra + clipboard. (Note: the ASR model doesn't clean text; the post‑process LLM does.)
4. **Phone‑as‑microphone** (likely last) — a local server in DotFlow serves a web page; the phone opens it on
   the LAN (QR pair), grants mic, **streams audio over WebSocket** to the desktop → existing transcribe
   pipeline. Browser‑based, no app‑store app.
5. **Weekly upstream sync** — `git fetch upstream && git log --oneline <last>..upstream/main`, cherry‑pick
   worthwhile fixes, rebuild+test. **Last synced: `0a59e1f`** (see `FORK.md`).
6. **Disable the noisy Handy CI workflows** (keep `code-quality` + `test`).

## Key files

- Rust product core: `src-tauri/src/dotflow/` — `phrases.rs` (expand + matching), `field_stream.rs`,
  `punctuation.rs`, `typed_expander/mod.rs`, `mod.rs` (`process_clause`, `starter_pack`, `wedge_table`).
- `src-tauri/src/managers/phrases.rs` (SQLite library) · `commands/phrases.rs` (CRUD).
- Injection: `src-tauri/src/clipboard.rs` (`inject_field_edit` char‑by‑char, `inject_bulk` clipboard,
  `is_injecting`/`InjectGuard`). Dictation stop path + `dictation-state` event: `src-tauri/src/actions.rs`.
  Field‑streaming hooks: `src-tauri/src/managers/transcription.rs`. Settings: `src-tauri/src/settings.rs`.
  Window/frameless/commands: `src-tauri/src/lib.rs`. Perms: `src-tauri/capabilities/default.json`.
- Frontend: `src/App.tsx` (view modes + resize + `dictation-state`), `src/components/DragonBar.tsx`,
  `TitleBar.tsx`, `Sidebar.tsx`, `settings/phrases/PhrasesSettings.tsx`, `settings/FieldStream*.tsx`,
  `icons/HandyTextLogo.tsx` (wordmark) + `HandyHand.tsx` (mark), `styles/theme.css` (emerald),
  `icon-source.png` (brand source → `bunx @tauri-apps/cli icon icon-source.png`).
