# DotFlow — fork notes (staying in sync with Handy)

DotFlow is a hard fork of **[cjpais/Handy](https://github.com/cjpais/Handy)** (MIT). We ride Handy's local
speech-to-text engine (Parakeet via `transcribe-rs`, Silero VAD, enigo injection, Tauri shell) and layer on
the DotFlow product: live in-field injection, dot-phrase / voice-alias macros, an editable phrase library,
and a Dragon-style UI. Handy's MIT `LICENSE` (© CJ Pais) is retained.

- **Forked from:** Handy commit `dad37baa0315c63c3a3d91af1fdcd6ad6e401f4f`.
- **Last synced with upstream:** `0a59e1f` (2026-07-07) — ported `0a59e1f` (ampersands in custom words)
  and `cdb4633` (overlay during post-processing); skipped `45e3eed` (Italian translations, we've rebranded).
  Next weekly check: `git fetch upstream && git log --oneline 0a59e1f..upstream/main`.
- **Upstream remote:** `upstream` → `https://github.com/cjpais/Handy.git` (our `origin` is this repo).
- Our git history was flattened at fork time, so `git merge upstream/main` is **not** a clean operation.
  We pull upstream improvements by **cherry-pick / hand-port**, which works across unrelated histories.

## CI on the fork

Handy ships heavy CI (signed builds, nix, playwright) that needs Handy's secrets or burns 30+ min per push.
On the fork we keep only the two fast, green workflows and **disable the rest via the GitHub API** (not by
editing the YAML — that would conflict on every upstream sync). Disabling this way leaves no trace in the
repo, hence this note.

- **Active:** `code quality` (~20s), `test` (~2min).
- **Disabled** (`gh workflow disable`, 2026-07-07): `main-build.yml` (fails — needs signing secrets),
  `nix-check.yml` (passes but ~36 min/push), `playwright.yml`, `build.yml`, `build-test.yml`,
  `pr-test-build.yml`, `release.yml`.
- **Check / re-enable:** `gh workflow list --all --repo cspergel/dotflow` · `gh workflow enable <file>.yml`.
- After an upstream sync **re-check this** — a synced change can re-add a workflow file or a new trigger; a
  freshly-added workflow starts **active** and must be disabled again if unwanted.

## Pulling an upstream change

```sh
git fetch upstream
git log --oneline upstream/main        # find the commit(s) you want
git cherry-pick <sha>                  # applies that commit's diff onto our tree
#   ...resolve conflicts (only in the files listed below) ...
# then rebuild + test:
cd src-tauri && CARGO_TARGET_DIR=C:/dtfb cargo test --lib dotflow && cargo build
```

If a change is large or entangled, read its diff and port the idea by hand instead of cherry-picking.

## Conflict surface — the ONLY files where upstream can collide with us

Everything else we added lives in **new files Handy doesn't have** (see below) and will never conflict. When
merging upstream, expect conflicts only here:

**Rust (`src-tauri/src/`)**

- `clipboard.rs` — char-by-char keystroke injection + settle (the Windows key-repeat fix); `inject_field_edit`; the **re-entrant** self-injection guard (`is_injecting`/`injection_guard`, an `AtomicUsize` counter) the typed expander reads; `paste()` runs the phrase wedge + reads the user's table.
- `managers/transcription.rs` — the streaming field-injection hook (`emit_stream_text`, `finalize_field_stream`, `FieldStreamer`, throttle/char-delay), reads the phrase table.
- `actions.rs` — emits `dictation-state` on record start/stop; the field-streaming stop path.
- `audio_feedback.rs` — added `SoundType::Expand` + `play_expander_sound` (the typed-expander ding).
- `settings.rs` — added `experimental_field_streaming`, `field_stream_throttle_ms`, `field_stream_char_delay_ms`, `experimental_typed_expander`, `typed_expander_sound`; `PasteMethod::Direct` default; default model = Parakeet.
- `lib.rs` — registers `PhraseManager` + phrase commands + the typed-expander controller/commands (boot-starts the monitor if enabled); frameless + smaller min window size; title "DotFlow".
- `managers/mod.rs`, `commands/mod.rs` — module registration for the new `phrases` + `typed_expander` modules.

**Config / build**

- `tauri.conf.json` — productName/identifier (com.dotflow.app).
- `capabilities/default.json` — added window perms (set-size/min-size/resizable/always-on-top/start-dragging/minimize/hide) for the compact bar.
- `Cargo.toml` / `Cargo.lock` — package rename `dotflow`, `default-run`, dropped whisper `vulkan` feature (x86_64-windows).
- `package.json`, `index.html`, `.gitignore`.
- `src-tauri/icons/*` — all regenerated from `icon-source.png` (the green dot-flow mark).

**Frontend (`src/`)**

- `App.tsx` — compact/full view mode + window resize; `dictation-state` listener; frameless chrome wiring.
- `components/Sidebar.tsx` — slimmed, de-branded (no wordmark, subtle active state).
- `components/icons/HandyTextLogo.tsx`, `HandyHand.tsx` — repurposed to the DotFlow wordmark + dot-flow mark (names kept for import compatibility).
- `components/settings/advanced/AdvancedSettings.tsx`, `components/settings/index.ts` — wire the field-stream toggles + Phrases page.
- `components/settings/about/AboutSettings.tsx` — source-code link → this repo.
- `i18n/locales/en/translation.json` — "Handy" → "DotFlow" (English only; other locales still say Handy).
- `styles/theme.css` — emerald palette (was Handy pink).
- `bindings.ts` — auto-generated by tauri-specta on a debug build; regenerate rather than merge (just build).

## DotFlow-only files (never conflict — safe to keep as-is on merge)

- `src-tauri/src/dotflow/` — `mod.rs`, `phrases.rs`, `punctuation.rs`, `field_stream.rs` (pure, unit-tested),
  and `typed_expander/` (`mod.rs` pure core + `backend.rs` Windows Raw Input keyboard monitor).
- `src-tauri/src/managers/phrases.rs` — SQLite `PhraseManager` (phrase library).
- `src-tauri/src/commands/phrases.rs` — phrase CRUD commands. `src-tauri/src/commands/typed_expander.rs` — the expander toggle + ding-toggle commands.
- `src-tauri/resources/expand.wav` — the typed-expander confirmation ding.
- `src/components/` — `DragonBar.tsx`, `TitleBar.tsx`, `settings/phrases/PhrasesSettings.tsx`,
  `settings/FieldStreaming.tsx`, `settings/FieldStreamThrottle.tsx`, `settings/FieldStreamCharDelay.tsx`,
  `settings/TypedExpander.tsx`, `settings/TypedExpanderSound.tsx`.
- `docs/dotflow-design/` — the product design docs (incl. `ROADMAP.md`, `SESSION-HANDOFF.md`).
- `icon-source.png` — the brand source (regenerate app icons with `bunx @tauri-apps/cli icon icon-source.png`).

## Keeping merges cheap

The rule that keeps this maintainable: **add in new files, touch Handy files minimally.** When a DotFlow
feature needs to hook into a Handy file, keep the edit to the smallest possible seam (a call into a
DotFlow-only module) rather than inlining logic — that shrinks the conflict surface above.
