# DotFlow — session handoff (resume here after compaction)

## What DotFlow is
A fork of **Handy** (`cjpais/Handy`, MIT, Tauri 2 + Rust + React) rebranded to DotFlow. It transcribes speech
with local Parakeet (ONNX/CPU via `transcribe-rs`). The DotFlow product layer = phrase expansion + punctuation +
live field injection on top of Handy's engine (design: `docs/dotflow-design/DotFlow-plan-v2.md` +
`DotFlow-asr-stack-research.md`).

## Environment / how to build + run
- Repo: `~/Documents/Coding Projects/dotflow` (a fresh Handy clone + our changes; **NOT its own git repo yet, uncommitted**).
- Toolchain: cargo 1.96 (msvc), node 22, bun 1.3, pnpm. VS 2022 C++ + Win SDK present. **No Vulkan SDK** →
  we dropped whisper's `vulkan` feature in `src-tauri/Cargo.toml` (we use Parakeet ONNX/CPU, not whisper-GPU).
- Build backend: `cd src-tauri && export PATH="$HOME/.cargo/bin:$PATH" && export CARGO_TARGET_DIR="C:/dtfb" && cargo build`  (short target dir dodges the Windows 260-char path limit).
- Run the app: `cd dotflow && export CARGO_TARGET_DIR="C:/dtfb" && bun run tauri dev`  (opens the window; long-running).
- Data dir: `%APPDATA%/com.dotflow.app/` (has the Parakeet model + `settings_store.json`). We copied the model +
  settings there from `com.pais.handy` when we changed the identifier.
- Toggle experimental flags by editing `settings_store.json` → `settings.experimental_*` (no UI toggle).

## Rebrand (DONE, keep)
`src-tauri/Cargo.toml` package `name = "dotflow"` + `default-run = "dotflow"`; `tauri.conf.json`
`productName: DotFlow`, `identifier: com.dotflow.app`; `package.json` name `dotflow`. Lib stays `handy_app_lib`.
→ distinct `dotflow.exe`, runs alongside Handy.

## Current state (updated 2026-07-06 — Steps 1 & 2 DONE, code-complete + green)
- **Batch dictation works** (Handy's proven path): Parakeet transcribes, overlay shows text, final text injects.
- **STEP 1 (cleanup) DONE.** Deleted `clause.rs` + `clause_worker.rs`; removed the `experimental_clause_injection`
  flag and all its wiring (audio.rs `clause_router`/`create_audio_recorder` param/audio-callback feed/
  `start_clause_session`/`finish_clause_session`; actions.rs record-start call + stop-path batch-skip branch).
  `cargo build` + `cargo test --lib dotflow` green (25 tests). No refs to any removed symbol remain.
- **STEP 2 (word-batched field-streaming) DONE (code) — needs a LIVE test.** `field_stream.rs` reworked:
  `advance(committed)` now HOLDS the trailing partial word and only releases text up to the last whitespace
  boundary (whole completed words), in far fewer/larger enigo bursts — the fix for the keystroke-race. Added
  `flush(committed)` to release the held word at stop; `finalize_field_stream` now calls `flush`. Removed the
  temporary `info!` diagnostic and the unused `reset_field_stream` wrapper (`start_stream` already resets
  inline at line ~781). 7 pure unit tests green (`cargo test --lib dotflow::field_stream`).
- **`experimental_field_streaming` is OFF** in the store — the app is in the clean, known-good batch state.
- **DotFlow product code** in `src-tauri/src/dotflow/`:
  - `phrases.rs`, `punctuation.rs`, `mod.rs` (`process_clause` + `starter_pack`) — **tested, KEEP.** The wedge
    (`.fix`/"insert follow up" → template; spoken punctuation). Wired into `clipboard.rs::paste` (batch path).
    0-survivor mutation-verified earlier. (Not yet in the STREAMING path — that's Step 3.)
  - Injection-first: `PasteMethod::default()` → `Direct` (enigo.text keystroke injection). **KEEP.**
  - `field_stream.rs` + the `emit_stream_text` hook (transcription.rs) — word-batched streaming into the field.
    **Reworked in Step 2; awaiting a live test with a streaming model.**

## HOW TO LIVE-TEST Step 2 (do this next)
1. Launch DotFlow: `cd dotflow && export CARGO_TARGET_DIR="C:/dtfb" && bun run tauri dev`.
2. Download a **streaming** model via the Models page — **Parakeet Unified** (the default `parakeet-tdt-0.6b-v3`
   is batch; field-streaming needs a model that emits committed/tentative). Select it.
3. In `%APPDATA%/com.dotflow.app/settings_store.json` set `settings.experimental_field_streaming: true`.
4. Focus **Notepad**, dictate a few sentences. Expect whole words to appear in the field a word behind your
   speech, cleanly — no `ssss rrrr` churn. The overlay still shows the live tentative guess.
- **If the race STILL appears** under rapid end-of-sentence commit bursts: the next lever is a small
  inter-write throttle (sleep ~10ms after each streaming enigo write, on the streaming worker thread in
  `emit_stream_text`) — NOT yet added, to keep the fix clean/tested first. That's the one-line fallback.

## Why the live-injection experiments failed (the lesson)
Both injected too aggressively: the clause worker mis-segmented (dots), and `field_stream` fires `enigo` on
*every* streaming update (many/sec) → rapid programmatic keystrokes lose a race with Windows input timing →
dropped/repeated keys (`ssss rrrr`). **The overlay is reliable because it just renders; hammering the keyboard
API is not.** The streaming DATA is perfect (overlay proves it) — the problem is the write mechanism.

## THE PLAN — start here (in order)

### Step 1 — CLEAN UP (remove the broken experiments)
- Delete `src-tauri/src/dotflow/clause.rs` and `clause_worker.rs`; drop their `mod`/`pub use` in
  `dotflow/mod.rs` (`clause`, `clause_worker`, `ClauseInjectionLoop`, `ClauseSegmenter`, `ClauseStream`, `ClauseRouter`).
- Remove `experimental_clause_injection` (settings field + the `get_default_settings` literal) and its wiring:
  `managers/audio.rs` (`clause_router` field + init + the `create_audio_recorder` param + the `.feed` in the
  audio callback + `start_clause_session`/`finish_clause_session`), and `actions.rs` (`rm.start_clause_session()`
  at record start + the `else if experimental_clause_injection { … }` batch-skip branch).
- KEEP `field_stream.rs`, the `experimental_field_streaming` flag, and the `emit_stream_text` hook — but rework
  the mechanism in Step 2.
- Verify: `cargo build` + `cargo test --lib dotflow` green; flags off ⇒ batch dictation still works.

### Step 2 — rebuild field-streaming (word-batched, the user's model)
The overlay already computes the correct accumulated committed text. Mirror **only completed words** to the field:
inject once per finished word (committed grew past the last word boundary), append-only, debounced — *one*
enigo write per word, not per chunk. Far fewer/larger writes ⇒ no keystroke-race churn. Keep `FieldStreamer`
(already append-only + tested) but only call it when a new **complete word** is available. Test into Notepad with
a **streaming model** (Parakeet Unified — needs downloading via the Models page). Remove the temporary `info!`
diagnostic in `emit_stream_text` once fixed.

### Step 3 — the specialized functions (on stable text)
Layer on the now-stable field text: (a) phrase expansion + punctuation in the streaming path (not just batch);
(b) "fix / clean up last dictation in place" (deterministic, then optional LLM) via `FieldStreamer.advance(cleaned)`;
(c) voice-edit ("scratch that", "correct X to Y"). Robust cursor-proof correction = the **accessibility-API**
path (design §11a) — a later per-OS upgrade.

## Also note
- The **DTF** project (separate repo, github.com/cspergel/DTF) got its comprehensive guide pushed
  (`docs/DTF-GUIDE.md`) — that work is DONE, unrelated to DotFlow.
- The installed upstream Handy was closed during testing; reopen from Start menu anytime — DotFlow is independent now.
