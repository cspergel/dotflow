# Selection → Review Overlay — design

> Status: **design agreed, ready to implement.** Date: 2026-07-07.
> Supersedes the "IN PROGRESS" sketch in [`../dotflow-design/SESSION-HANDOFF.md`](../dotflow-design/SESSION-HANDOFF.md) §"the next task".

## Goal

A second global hotkey grabs the current selection and pops a small **always-on-top
DotFlow card near the cursor** showing a review/action surface. The user accepts fixes (or runs
an AI action) and the result is pasted back into the field they came from — a **single `Ctrl+Z`
reverts it**. This is the "edit where I am" answer: a floating card, *not* literal in-app editing.

The feature is **toggleable** (master on/off in settings, default **on**) and the hotkey is
rebindable. It is **additive and non-intrusive**: it never changes your text until you say so,
and `Esc` always dismisses with zero changes.

## Scope

**In (v1):**
- New rebindable hotkey `review_selection` (default `Ctrl+Shift+J` — **not** Ctrl+Alt+\*, which is AltGr).
- Master setting `selection_review_enabled` (default **on**).
- Cursor-anchored, always-on-top, frameless overlay window (pre-created hidden at startup).
- **Default action = Proofread** (Harper, offline, instant) rendered in the existing `ReviewPanel`,
  shown *immediately* on open — zero extra clicks for the common case.
- **Action chips** in the same card: `Proofread ·offline` │ `Rewrite` `Formal` `Summarize`.
  The three AI chips are **disabled with a hint** when post-process is not configured
  (`post_process_is_configured() == false`), so the offline experience never looks broken.
- Apply pastes back via the existing clipboard-injection path; `Esc`/click-away cancels.

**Out (deferred, intentionally):**
- **Post-dictation review** (pop the same card after a dictation finalizes). Built on the *same*
  window later, shipped **off behind a setting** — low value while dictation lands near-live, so
  no dead UI in v1.
- Fixed-position ("command bar") mode as an alternative to cursor-anchoring — possible later setting.
- Auto-`Ctrl+A` when nothing is selected (risky across apps) — v1 **requires a selection**.
- Bundled local LLM — see "Future AI" below. v1's AI chips use the existing Ollama-or-nothing path.

## Design

### 1. Architecture & flow

Mostly a *window + plumbing* job; the review panel and cleanup engine already exist.

1. Hotkey `review_selection` fires (gated by `selection_review_enabled`).
2. Action mirrors the **copy phase** of the existing cleanup hotkey: `wait_for_modifiers_released()`
   → sentinel-based synthetic `Ctrl+C` → read selection from clipboard → **restore clipboard**.
   (Reused verbatim from `actions.rs` / `input.rs`.)
3. Empty selection → **no-op** (v1 requires a selection).
4. Capture the source window handle (`GetForegroundWindow()`) and the mouse position
   (`GetCursorPos`) **before** showing anything.
5. Position the pre-created overlay window at the clamped cursor location, show it, emit
   `review://open` with `{ text, aiAvailable }`.
6. Overlay renders the action-chip shell + `ReviewPanel` (Proofread result already computed).
7. Apply → `apply_review_result(text)` command → refocus source → `clipboard::inject_bulk`
   (guard + clipboard restore) → single paste. `Esc` → hide, nothing changed.

### 2. Focus & paste-back (the reliability crux)

Getting focus right is what makes it feel seamless vs. broken.

- **At hotkey time, before showing anything**, stash `GetForegroundWindow()` — "the field I came from".
- The overlay shows as a **normal focusable window** so keyboard nav is first-class (arrow between
  chips, Enter = Apply, Esc = cancel). Taking focus is fine *because* we remembered the source.
- **On Apply:** hide overlay → `SetForegroundWindow(saved_hwnd)` → **poll until it actually regains
  foreground** (don't guess) → run the existing `inject_bulk` paste (set clipboard to result →
  synthetic `Ctrl+V` → restore original clipboard). Net: one paste ⇒ single `Ctrl+Z` reverts.
- **On Esc / click-away:** just hide. Clipboard was already restored in the capture phase.
- **Windows caveat (do not re-derive):** `SetForegroundWindow` has foreground-lock rules; here the
  user *just* interacted with our process via the hotkey, so the OS normally grants it. Add an INFO
  log at the refocus point (per the silent-async-panic gotcha) and treat "did it actually refocus"
  as a build checkpoint. **Fallback if flaky:** a `WS_EX_NOACTIVATE` overlay that never takes focus
  (mouse-only), keeping the source field focused throughout.

### 3. Overlay window & UI

- **Window:** dedicated frameless, always-on-top, auto-sizing Tauri window with its own entry point
  (precedent: `src/overlay/`). **Pre-created hidden at startup** so popping is instant (cold-creating
  per press adds visible lag).
- **Text routing:** backend positions + shows the window, then emits `review://open` carrying
  `{ text, aiAvailable }`. The window listens, runs `analyze_text` (Harper) on mount, renders.
  Apply calls `apply_review_result` with the final string.
- **Layout (top → bottom):**
  - **Action chips:** `Proofread ·offline` │ `Rewrite` `Formal` `Summarize`. Proofread selected and
    already showing its result on open. AI chips disabled + hinted when `aiAvailable == false`.
  - **Body:** Proofread → the existing `ReviewPanel` (underlined issues, click-to-accept, per-issue
    cards) reused as-is. AI action → a simpler before→after result view with a spinner while the LLM
    runs (slower, opt-in).
  - **Footer:** `Apply` (primary), `Copy`, `Close`. Enter = Apply, Esc = Close.
- **Sizing:** ~420px wide, max-height with internal scroll, clamped on-screen (flip above/left near edges).
- One `ReviewPanel` serves three surfaces: Try-it box (done), this overlay, and later post-dictation.

### 4. Settings & wiring

- **Settings:**
  - `selection_review_enabled` (bool, default **true**) — master on/off.
  - `review_selection` shortcut binding (default `Ctrl+Shift+J`, rebindable, modifier-validated).
  - No post-dictation setting in v1.
- **Settings UI:** lives in the existing **Cleanup** section (sidebar → REVIEW group), under the
  current cleanup hotkey — master toggle + rebindable review hotkey + a one-line "requires AI
  configured for Rewrite/Formal/Summarize" note. No new sidebar entry.
- **Backend wiring** (paths per the handoff's Key Files):
  - Add `review_selection` to `ACTION_MAP` + a handler in `actions.rs` (copy phase reused).
  - New command `apply_review_result(text)` → refocus saved HWND → `inject_bulk`.
  - Register/validate the binding in `shortcut/tauri_impl.rs`; defaults in `settings.rs`
    `get_default_settings`.
  - **Hand-add** the new commands/types to `src/bindings.ts` (specta only regenerates on a real app
    run — known gotcha).
- **i18n:** every new UI string added to **all 21 locales** (CI translation check requires every key
  in every locale — use the add-key script, English as source).

### 5. Testing (real teeth — Design Truth Flow charter active)

**Unit (`cargo test --lib`) — pure logic, real assertions incl. boundaries:**
- `place_overlay(cursor, window_size, screen_bounds) → (x, y)`: asserts it flips above/left exactly
  at the boundary (cursor near right/bottom edge), stays put mid-screen, and never returns
  off-screen coordinates. Includes a case that *would* overflow without clamping, so the test fails
  if the logic breaks.
- Empty-selection → no-op decision; AI-availability gating (`aiAvailable == false` ⇒ AI actions
  disabled) — small pure predicates, true + false case each.

**Proven by exercising the app (`verify` / `run` skill — not fake unit tests):**
- Focus save → refocus → single paste, and that one `Ctrl+Z` reverts it. Crosses the OS boundary
  (`GetForegroundWindow` / `SetForegroundWindow` / synthetic keys) — a unit test here would be
  theater. Drive it live in a real field, INFO log at the refocus point confirms it refocused.
- Window pops at cursor, chips switch, Apply pastes back.

**Frontend:** chip shell renders AI chips disabled when `aiAvailable == false`; `ReviewPanel`
already validated.

We will **not** claim the focus/paste path "passes" from a green unit run — only after watching it
work end-to-end.

## Key files (per handoff)

- Actions/hotkey: `src-tauri/src/actions.rs`, `input.rs`, `shortcut/tauri_impl.rs`, `settings.rs`.
- Injection: `src-tauri/src/clipboard.rs` (`inject_bulk`, `injection_guard`).
- Grammar/analyze: `src-tauri/src/dotflow/grammar.rs`, `commands/cleanup.rs` (`analyze_text`,
  `post_process_is_configured`).
- Overlay precedent: `src/overlay/` + the frameless main window.
- UI: `src/components/settings/cleanup/` (`CleanupSettings.tsx`, `ReviewPanel.tsx`), new overlay entry.
- Bindings: `src/bindings.ts` (hand-add). i18n: `src/i18n/locales/*/translation.json`.

## Gotchas to respect (from handoff — do not re-derive)

- Wait for trigger modifiers to release before the synthetic `Ctrl+C` (else Ctrl+Shift+C copies nothing).
- Never bind Ctrl+Alt+\* (AltGr on Windows).
- A panic in a `tauri::async_runtime::spawn` task vanishes silently — wrap Harper calls, log at fire points.
- Shortcut validator rejects modifier-less global bindings.
- Kill the running app before relaunching a rebuilt binary (single-instance forwarding; DLL lock on tests).
- specta bindings regenerate only on a real app run.

---

## Future AI (captured 2026-07-07 — NOT v1 scope)

Research done this session on making the AI actions (Rewrite / Formal / Summarize, and a deeper
proofread) run **seriously, fast, and locally on an everyday computer without a GPU**. This is a
later milestone; v1 ships with Harper (Proofread) + the existing Ollama-or-nothing AI path.

> ⚠️ Some releases below post-date the Jan 2026 knowledge cutoff and come from live web research —
> **verify version numbers and licenses against the actual model cards before committing.**

### Engine

- **Use `llama-cpp-2` (llama.cpp Rust bindings).** Decisive reason: DotFlow **already ships a
  GGML/GGUF stack** (`transcribe-cpp` for whisper) and pays its C++/CMake build cost — so adding
  llama.cpp is cheap, gives one weights format (GGUF) + one kernel stack for both speech and text,
  the fastest CPU kernels, and **static linking → no external dependency.**
- **CPU-only by default; Vulkan/Metal as opt-in per-platform flags** (mirrors STT).
- **Reuse `managers/model.rs`** to download the ~1–2GB GGUF on first AI use (don't bloat the installer).
- **Keep Ollama as an optional power-user HTTP backend** (detect `localhost:11434`), never a
  dependency. It's llama.cpp underneath, so behavior stays consistent — and it's where a big-GPU
  user runs Gemma 4 32B etc.
- **Don't** stretch `ort`/ONNX into an LLM decoder (no generation/KV-cache layer); keep `ort` for STT.
- LiteRT-LM (Google's on-device runtime, used by the *parlor* project) considered — Gemma-specific /
  Python-oriented, worse fit than llama.cpp for our Rust/Tauri + existing-GGUF setup.

### Model ladder (all commercially clean — Apache-2.0 / MIT)

Ship **two tiers behind one setting**: a fast small default + an optional higher-quality model.

| Model | Size | License | Notes |
|---|---|---|---|
| **Gemma 4 E2B / E4B** | ~2.3B / ~4.5B eff. | **Apache-2.0** | On-device-designed (per-layer embeddings); user has used the family and rates it. Q4 ≈ 3GB. |
| **Qwen3-4B-Instruct** (or newer Qwen3.5) | ~4B | **Apache-2.0** | Best quality-per-byte; use *non-thinking/instruct* variant for latency. Q4 ≈ 2.5GB. |
| **Phi-4-mini** | 3.8B | **MIT** | Cleanest license; strong at formalize/summarize (stiffer for creative rewrite). |
| Qwen3-1.7B / Gemma small | 1.7–2B | Apache-2.0 | Fast default tier (~1–5s CPU for short output). |
| SmolLM3-3B, IBM Granite 4.x | ~3B | Apache-2.0 | Fully-open / enterprise fallbacks. |

**Avoid for a sold product:** Llama 3.2 (Community License — 700M-MAU cap, competitor ban, naming
requirement); Gemma **3 / 3n** (restrictive custom license — only **Gemma 4** is Apache-2.0);
Qwen2.5-**3B** specifically (non-commercial Qwen Research License — the 0.5/1.5/7B sizes are Apache).

**CPU viability (no GPU, short outputs):** 1.7–2B Q4 ≈ 1–5s; 3–4B Q4 ≈ 3–15s. Fine for an on-demand
button; not for live streaming. Two-tier default handles both.

**Vetted community links (result of this session's research):**
- ✅ `unsloth/gemma-4-E2B-it-GGUF` — the keeper (Apache-2.0, CPU-designed).
- ❌ `Merlin-Research/Merlin-Agent` (9B coding agent, wrong task, "quantum" marketing).
- ❌ `squ11z1/Mythos-nano` (Qwen2.5-3B non-commercial base mislabeled MIT; abliterated).
- ❌ `King3Djbl/nexus-medical-GGUF` (uncensored, unvalidated, anonymous — unsafe for clinical).

### Grammar beyond Harper

The cleanup engine is a ladder: deterministic ✅ → **Harper** ✅ → **ML-GEC** (CoEdIT/GECToR exported
to ONNX, runs on the `ort` runtime already shipped — no new engine) ⏳ → **LLM** ✅. For *better
grammar* you don't need an LLM; ML-GEC on `ort` is the next rung.

### Medical beachhead — safety guidance (important)

- **Do NOT bundle a medical-finetuned LLM as a clinical tool.** The good ones are research-licensed
  (Meditron) or carry non-Apache terms + medical-device regulatory risk (MedGemma). The task is
  *cleanup/summarize*, where the cardinal sin is **inventing clinical facts** — a med-finetune can
  make that worse and pushes toward FDA device territory.
- **Safe architecture:** general Apache/MIT small model (low-temp, "add no facts" prompting,
  clinician always reviews) **+ a medical terminology pack in the deterministic Harper/Hunspell
  layer** (deterministic, auditable, zero hallucination).
- ⚠️ **License gotcha:** popular Hunspell medical wordlists (glutanimate, ~90k terms) are **GPL-3.0**
  — do not compile into a proprietary binary. Source a permissive list, or ship a GPL pack only as
  separate user-installed data (legal review).
- **Framing:** "clinician-reviewed cleanup, not decision support" keeps DotFlow in the FDA
  enforcement-discretion safe zone.

### Spoken output — deferred, not on the map

Kokoro-82M (Apache-2.0, 82M params, CPU-viable, has ONNX builds → would run on the shipped `ort`)
noted as a clean option **if** a read-aloud / talk-back capability is ever wanted. Not planned now.
