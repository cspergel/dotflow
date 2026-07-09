# DotFlow — Roadmap

Working order agreed for the current arc. Check items off as they land; keep this file the single
source of truth for "what's next". Full context for each item lives in
[`SESSION-HANDOFF.md`](./SESSION-HANDOFF.md).

## Order

### 1. Typed text expander — backend (IN PROGRESS)

Type a dot-trigger (`.fu`) in **any** app → it's replaced by the saved phrase (the same library that
powers spoken triggers). Step 1 (safe foundation, off by default) is **done** (`8c4eb61`).

- [x] Step 1 — setting `experimental_typed_expander` (default OFF), self-injection suppression
      (`is_injecting()` / `InjectGuard`), pure tested core (`ExpanderBuffer`, `match_typed_trigger`).
- [x] Step 2/3 — Windows Raw Input keyboard monitor **implemented + live-validated**
      (`dotflow/typed_expander/backend.rs`). Smoke-tested by driving synthetic keystrokes into a separate
      window: typing `.fix` erased the trigger and pasted "Fix the bug where " (clipboard preserved), and a
      lone sentence period was left literal. Caught + fixed a real bug along the way (empty-key phrases made a
      bare `.` trigger — see below).
  - `RegisterRawInputDevices` (keyboard, `RIDEV_INPUTSINK`) + message-only hidden window + `WM_INPUT`
    pumped on a **dedicated native thread**; chars decoded with `ToUnicodeEx` + a tracked 256-key state.
  - Feed printable chars → `push`; Backspace → `backspace`; Enter/Tab/Esc/arrows/nav/Delete → `reset`;
    modifiers/lock/function keys → ignore (don't wipe an in-progress trigger).
  - On `matched()`: if `!is_injecting()`, raise `injection_guard()` → erase the `.key` via
    `inject_field_edit(n, "")` → paste the expansion via `inject_bulk` → `consume(n)`; a ~300 ms trailing
    **settle** window swallows the async `WM_INPUT` echoes of our own keystrokes so it can't re-trigger.
  - Injection guard made **re-entrant** (counter) so the outer emit guard + inner paste guard nest safely.
  - Start/stop the monitor thread on the setting toggle (`change_typed_expander_setting` command) and at
    boot when already on; UI toggle in Advanced → Experimental with an explicit "monitors your typing" note.
  - Windows-only; a `#[cfg]` stub keeps other platforms compiling (mac/Linux backends can slot in later).
  - **Bug fixed (found via smoke-test):** alias-only phrases are stored with `key = ""`, so `match_typed_trigger`
    built the trigger `".{key}" == "."` and expanded on **every lone period**. `PhraseTable::new` now drops
    empty keys (with a defensive guard in `match_typed_trigger`), fixing both the typed and spoken paths.
  - **Deviations from the original spec:** backspaces go through the tested `inject_field_edit` (enigo)
    rather than raw `SendInput`; self-suppression uses the guard + a time-based settle rather than HID-source
    filtering. **Known v1 limitation:** a trigger fires the instant it completes (`.fu` expands mid-word in
    `.fund`) — pick trigger keys that aren't word prefixes.

### 2. Housekeeping ✅

- [x] Disabled the noisy Handy CI workflows via `gh workflow disable` (not YAML edits, to avoid upstream-sync
      conflicts): `main-build`, `nix-check`, `playwright`, `build`, `build-test`, `pr-test-build`, `release`.
      Kept `code-quality` + `test`. Documented in `FORK.md` (§CI on the fork) with the re-enable command.
- [x] Upstream sync check (2026-07-07): `upstream/main` head is `0a59e1f` — the last sync point, so **no new
      commits to port**. Re-run `git fetch upstream && git log --oneline 0a59e1f..upstream/main` next week.

### 3. Premium redesign (expanded window) — good pause point (more polish possible later)

- [x] Surface-ladder look (off-white canvas → white panels → inset controls, green-biased hairlines, no
      shadows), Geist bundled font, emerald reserved for meaning, refined toggles/titlebar/inputs.
- [x] DotFlow-specific IA: grouped sidebar (DICTATE / REVIEW / SYSTEM) with Phrases elevated + emerald rail.
- [x] Trimmed model catalog (Parakeet V3/V2 · Whisper Small/Turbo/Large-v3 · Moonshine V2 streaming).
- [x] Super-compact "mini" bar tier; removed Handy donate + acknowledgements.
- [ ] _Optional future polish:_ deeper visual refinement (the design can still improve — paused here by
      choice to move to task 4). Remaining pages still on old tokens: History, Post-processing, Phrases,
      General, Debug (they work; not yet ladder-styled).

### 4. Clean-up-selected-text hotkey ✅

- [x] `cleanup_selection` hotkey (default **Ctrl+Shift+U** — NOT Ctrl+Alt+\*, which is AltGr on Windows and
      gets swallowed). Copies the selection → cleans it → pastes over it → restores the clipboard. One-shot
      `CleanupSelectionAction`; sync clipboard/keystroke work on a blocking thread, only the LLM awaited.
- [x] **Deterministic cleanup by default** (`dotflow/cleanup.rs`, 10 unit tests): whitespace, spacing around
      punctuation, sentence capitalization, "i"→"I". Conservative — preserves decimals/number groups/line
      breaks; leaves ambiguous grammar (its/it's) alone. Zero setup.
- [x] **Upgrades to the post-process LLM** when configured (local Ollama or cloud) via a fixed built-in
      cleanup prompt — reuses `post_process_transcription` with a settings clone, so the dictation path is
      untouched. Falls back to deterministic if the LLM isn't configured or fails.
- [x] **Footgun fix:** the shortcut validator now rejects modifier-less global bindings (a bare `l` was
      firing on every keypress). Escape + F-keys stay exempt. 5 unit tests. (`shortcut/tauri_impl.rs`)
- Live-validated: `hello  world ,its  me` → `Hello world, its me`.

### 5. Offline grammar cleanup + Grammarly-style review (IN PROGRESS)

Free, fully-offline grammar/spelling via **Harper** (`harper-core`, Apache-2.0) — no API key. The engine is
in and wired to the Ctrl+Shift+U hotkey (see commit `e7f8a75`).

- [x] Harper engine (`dotflow/grammar.rs`): `harper_cleanup` (auto-apply confident fixes) + `analyze`
      (return spans/kind/message/replacements for a review UI). Panic-safe. Cleanup tier order:
      post-process LLM → Harper → deterministic tidy. Unit-tested.
- [x] **Cleanup** settings section (hotkey, engine indicator, "Try it" box via `preview_cleanup`).
- [x] Hotkey reliability: wait for modifier release before the synthetic Ctrl+C, sentinel-based copy detect.
- [x] Moved the typed-expander + ding toggles out of Advanced→Experimental into the **Phrases** section.
- [ ] **Review panel** (NEXT): a Grammarly-style UI in DotFlow's own window — text with issues underlined,
      click-to-accept/reject each suggestion, then insert/paste. Data layer done (`analyze_text` command +
      `TextSuggestion`); build the panel UI (render text + highlight `start..end` spans + replacement
      chips + apply by char-splice). Feed it from: (a) **post-dictation** review, and (b) a **selection**
      hotkey (selected text or whole field).
- [ ] **Live trailing autofix** (later, experimental): correct completed words/sentences as you type in any
      app, reusing the typed-expander's Raw Input monitor + focus-change stop. No highlighting.
- [ ] **Dictionaries**: Medical/Legal wordlist toggles on top of Harper (`hunspell-en-med-glut`, OpenMedSpel).
- [ ] _Deferred:_ in-app highlighting _inside other apps_ (Gmail/Outlook) — needs per-app accessibility /
      overlay integration (the hard Grammarly path). Revisit only if we commit to it.

## AI actions on the local model — prioritized (2026-07-08)

Now that the local-LLM runtime + `ai_transform` seam + model picker exist (branch
`feat/selection-review-overlay`), these are the next capabilities. **Anti-bloat principle:** do NOT add a
chip per action. Instead evolve the selection popup into **one command surface** — a single "type or dictate
what to do" input + 2–3 pinned quick actions + a small context-aware "more…" list. That input *is* the
custom-instruction box, *is* the dictation-command entry point, and scales to unlimited actions with zero
clutter. Every action below is a prompt behind that surface, not a new button.

> **Gate:** do NOT implement any of this until the prior work (overlay + local AI + Nemotron on this branch)
> is live-tested and merged.

- [x] **P1 — Command surface + custom instructions** (foundation). Shipped: the review card now has a
      "type or say what to do…" input (dictation via the mic) that runs a free-form instruction against the
      selection (backend `ai_transform_custom` → `build_custom_system`, model-direct routing = design option
      A), alongside the retained pinned chips (Proofread/Rewrite/Formal/Summarize per user request). Deferred
      to P2/P3: context-suggestion chips, the "more actions" popover, user-configurable pins, and the tiny
      intent classifier (option B). Spec: `docs/plans/2026-07-08-command-surface-design.md`.
- [ ] **P2 — Named actions as prompts** (low effort each, on the existing seam): **Translate** (offline —
      biggest differentiator; dictate/pick target language) first, then Tone presets (one "Tone ▾", not 5
      chips), Expand, Extract, Reply. Plus a **"Structure → SOAP note"** action — the highest-value one for
      the medical beachhead (freeform dictation → Subjective/Objective/Assessment/Plan).
- [ ] **P3 — Dictation command mode** (moderate–high; the on-brand one): a hands-free flow — mode/wake-word →
      dictate a command → acts on the selection/field → inserts. Tiny model routes intent, small model (Gemma)
      executes. Agent-lite, dictation-focused.
- [ ] **P4 — Vision** (high; model-dependent — DEFERRED): screenshot a region → summarize/extract → insert.
      Gemma 4 E2B ships vision (`mmproj-*.gguf`) and llama.cpp supports multimodal, but `llama-cpp-2` needs
      the multimodal (mtmd) path wired. Park until P1–P3 land.

## Known bugs (fix soon)

- **Review overlay wedge** (`feat/selection-review-overlay`): once the review card is open it doesn't dismiss
  on click-away, so re-pressing the hotkey `raise`s the already-open card AND still runs `copy_selection` —
  but the now-focused overlay window has no selection, so the copy returns empty and the card stays stuck
  empty. Fix: when the overlay is already open, raising it must **not** attempt a fresh self-copy (skip the
  copy, or copy from the previously-focused window), and/or give the card an obvious close affordance +
  auto-dismiss option. Observed live 2026-07-08 during GPU testing.

## Later / backlog

- Phone-as-microphone (LAN web page, QR pair, WebSocket audio → transcribe pipeline). Likely last.
- **Remote control (companion to phone-as-mic).** A phone/web client that *drives* the desktop app over the
  LAN — the inverse of phone-as-mic. Shared plumbing: a small local server on the desktop + QR/token pairing +
  a browser client. Capabilities: **chat with the local GPU-backed model from your phone** (desktop does the
  inference), trigger dictation / AI actions / macros, pull transcripts. Local-only + paired + no cloud
  (privacy thesis); a secure tunnel for true "anywhere" access is a later add. Model it on Claude Code's
  remote-control UX (a lightweight remote session). P3/agentic territory; build alongside phone-as-mic since
  they share the server + pairing + client. (Idea: 2026-07-09.)
- **Whole-window AI action** (extends the highlight→hotkey→action primitive): a hotkey/mode that sources the
  text from the **entire focused window** instead of a selection — grab via select-all+copy, or UI Automation
  (`UIA`/accessibility) to read read-only/browser content — then run the same AI action (Summarize, etc.) and
  show the result in the review card. Pairs naturally with the command surface (P1).
- **GPU CUDA runtime — auto-download on GPU-enable** (the product-grade delivery for GPU accel). Ship the
  small CPU build; when a user turns on GPU acceleration, download NVIDIA's redistributable
  (`cudart`/`cublas`/`cublasLt`, ~515 MB) into `%APPDATA%/…/cuda/`, verify, and add that dir to the DLL
  search path so the app finds it at next launch — no 700 MB in the installer, no dependency on a
  pre-installed CUDA Toolkit. Supersedes today's interim (dev machine: static-CUDA build that *locates* the
  already-installed toolkit via a folder-local launcher — see `SESSION-HANDOFF.md`). Note: this does NOT slim
  the exe itself (~122 MB from statically-linked CUDA kernels); that only shrinks with the parked
  dynamic-`ggml-cuda.dll` approach, which is blocked by the whisper/llama `ggml-base.dll` collision (research
  2026-07-08) and would need a separate CUDA helper process. Gate behind the same runtime GPU toggle.

## Document ingestion — PDF → summarize / ask (2026-07-09)

Drop a PDF into the AI Chat → extract its text locally → summarize it, pull key points, or ask questions
about it (fully offline; the local model's large context holds a whole document). Useful for referrals,
discharge summaries, lab PDFs, articles; composes with the EMR agent later (the agent can hand a chart PDF to
this same pipeline). **Staged:**

1. **Text-based PDFs (IN PROGRESS — prototyping now).** Pure-Rust text extraction (`pdf-extract` crate) →
   feed the text to the existing local-model chat as context. No OCR. Immediate value for digital PDFs. A
   scanned/image-only PDF yields no text layer → surface a clear "looks scanned, OCR coming" message.
2. **Scanned PDFs via ONNX OCR (next).** The key insight: **DotFlow already ships ONNX Runtime + DirectML**
   (Parakeet STT + the `ort` accelerator). So run OCR through the runtime we already bundle — **no Python**.
   Best fit = **PaddleOCR PP-OCRv5 models exported to ONNX**, run via `ort` in Rust, GPU-accelerated on the
   same onnxruntime → PaddleOCR-class accuracy on printed scans with clean packaging. `Tesseract` (native,
   `leptess`) is the quick lower-accuracy fallback. Handwriting is where traditional OCR fails — that needs 3.
3. **VLM upgrade (frontier / composes with P4 vision).** A vision-language model reads a scanned page AND
   summarizes in one model call — no separate OCR stage. **Key update (2026-07-09): the user's Qwythos-9B
   ALREADY has vision** — pair it with its `mmproj-*.gguf` (CLIP-style encoder + projector; its vision tower
   is inherited from base Qwen3.5-9B, frozen during the text-only SFT, so vision ≈ base Qwen3.5-9B
   multimodal). So the VLM path is **not blocked on a new model** — only on the integration. **Two integration
   paths:**
   - **(a) llama-server sidecar (pragmatic — routes around the mtmd blocker).** `llama-server -m <quant>
     --mmproj <mmproj>` exposes an **OpenAI-compatible vision API** (`/v1/chat/completions` with base64
     images). DotFlow **already speaks that shape** (the cloud/Ollama post-processor). Cost: an extra process
     + a second model load in VRAM (~tight alongside chat on a 16 GB 5080 → swap models). Fastest route to
     working vision/OCR.
   - **(b) in-process mtmd binding.** Wire llama.cpp's multimodal (`mtmd`) path into `llama-cpp-2` (the P4
     blocker). Cleaner (one process, shared model load) but real binding work.
   - **Model choice: start with Qwythos's own vision** — one model for chat + OCR, no extra download/VRAM,
     Qwen3.5-VL-class is good at *printed*-doc OCR. Add a **dedicated OCR VLM** (DeepSeek-OCR / Baidu
     Unlimited-OCR — one-shot long-doc parse incl. layout/tables) **only if** real docs demand it (dense
     tables, small print, handwriting). That slots cleanly into the **per-task-models** system as an `"ocr"`
     role with its own optional model override. Digital PDFs need no vision model at all (option 1).

**OCR tool survey (2026-07-09, for a Rust/Tauri offline app):** Tesseract = C++/native, Rust-bindable,
bundles cleanly, weak on messy scans (Apache-2.0). EasyOCR = Python/PyTorch (heavy runtime → ruled out as a
bundled dep). PaddleOCR = Python, **but models export to ONNX** → the native path above (Apache-2.0).
Unlimited-OCR / DeepSeek-OCR = VLM, Python/PyTorch/NVIDIA, MIT — most capable (layout/tables), the option-3
frontier. **Decision: prefer ONNX (reuse the shipped runtime) over any Python dependency; VLM via llama.cpp
is the long-term end state.** Open input needed: how much of the user's scanned volume is handwritten (pushes
option 3 sooner) vs printed (option 2 suffices).

## Long-term vision — voice-driven local agent for a web EMR (exploratory, 2026-07-09)

The most ambitious direction: a **local tool-calling agent** that automates clinical busywork (chart-prep,
note drafting) in a **simple web-based EMR**, driven by voice from the mic or phone. It composes DotFlow's
existing primitives — offline ASR in, local GPU LLM reasoning, the P3 dictation-command mode, and the
remote-control backlog — into an agent loop. Flagged **exploratory**: build only after the core dictation +
AI-actions arc is live and merged, and only against an EMR the user is authorized to automate.

**The load-bearing design decision — split "what" from "how".** A small local model is *not* trusted to guess
the DOM live (brittle + unsafe near a chart). Instead:

- A hand-cleaned **site adapter** owns the *how*: a stable map of this EMR's UI to named, audited actions
  (`open_chart`, `get_last_visit`, `fill_note(text)`, `save`), split **read** vs **write**. Deterministic.
- The **tool-calling model** (Hermes / Qwen function-calling) owns the *what*: it emits `fill_note("…")`
  calls; it never touches selectors. It supplies the variable content (which patient, the note text); the
  adapter supplies the frozen mechanics.
- A **confirmation gate** wraps every *write* action (voice or tap "yes"). Non-negotiable — a wrong write in
  a live chart is a patient-safety event, not a bug.

**Build the adapter by demonstration, not by hand (record-to-teach).** Use `npx playwright codegen <emr-url>`
(or Chrome DevTools → Recorder): the user performs one real chart-prep, the recorder captures the steps +
selectors, and we clean that into the adapter. Productized later as a DotFlow **"Teach mode"** (demonstrate a
workflow once → saved as a reusable, voice-triggerable action) built on the same CDP recording.

**Two questions to resolve before scoping (cheaper paths hide here):**

- **Hosted-with-login or local?** Decides the driver + the auth/pairing story for the phone trigger.
- **Is there an HTTP/JSON API behind the page?** (DevTools → Network while saving a note.) If yes, driving
  those authenticated calls directly is *far* more robust than UI automation and **skips the browser adapter
  entirely** — a "simple webpage EMR" often has a trivial JSON backend. Prefer API-over-DOM whenever it exists.

**Safety invariants (all phases):** PHI never leaves the machine (local model only); the phone/remote channel
is paired + authenticated, never open LAN; writes are always human-confirmed; start **draft-only** and add
"acting" tools one at a time, each behind confirmation, each reversible.

**Implementation steps (phased, each shippable + gated):**

1. **Recon + decision (no code).** Answer hosted/local + API-or-DOM. If an API exists, target it and skip
   steps 2–3's browser work in favor of a typed API client with the same read/write split.
2. **Driver + one read tool.** Attach to Chromium over CDP (Rust `chromiumoxide`, or a Playwright sidecar).
   Prove `get_chart()` / `get_last_visit()` against the real EMR (read-only, zero risk).
3. **Site adapter from a recording.** `playwright codegen` one chart-prep → clean the selectors into the
   adapter → expose the read actions + a single write action `fill_note(text)`. Unit-test the adapter's
   selector resolution against a saved copy of the page.
4. **Tool registry + agent loop.** Wire the adapter actions as tools for the local tool-calling model
   (Hermes/Qwen); bounded step count; structured call → observe → next. Read tools auto-run; **write tools
   pause for confirmation.** Reuse the P3 dictation-command intent router as the front door.
5. **Draft-only MVP.** Voice command → agent reads the chart, drafts a **SOAP note** (reuses the P2
   "Structure → SOAP" action), fills it into the note field, and **stops before Save** for review. This is
   the 80% time-saver with none of the autonomous-write risk.
6. **Voice + phone trigger.** Hook the loop to the mic and the remote-control channel (shared server +
   pairing from the phone-as-mic / remote-control backlog) so it's runnable hands-free / from the phone.
7. **Guarded "acting" tools (last).** Add `save` and any other writes strictly behind explicit per-action
   confirmation, with an audit log of every action taken. Never autonomous.

**Depends on / composes:** P2 (SOAP action), P3 (dictation command mode / intent routing), and the
remote-control + phone-as-mic backlog (shared local server + pairing). A new local **tool-calling model**
(Hermes or a Qwen function-calling variant) is the added dependency — importable via the existing GGUF model
picker. Detailed adapter-pattern + record-to-teach sketch to live in `docs/plans` when this leaves the
backlog.
