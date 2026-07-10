# DotFlow — session handoff (resume here in a fresh session)

> Last updated end of the **2026-07-09** session. Current work is on branch **`feat/ai-chat-gpu`**
> (~18 commits ahead of `main`, all pushed to `origin`, **not merged**). Read this + [`ROADMAP.md`](./ROADMAP.md)
> to pick up. The detailed running log is in the auto-memory `dotflow-state.md`.

## What DotFlow is

A fork of **Handy** (`cjpais/Handy`, MIT; Tauri 2 + Rust + React) rebranded to **DotFlow**. Local-first,
**fully offline, privacy-first** dictation + text tooling. Differentiators: live in-field dictation (Dragon
feel), dot-phrase / voice-alias macros, a typed text expander, an editable phrase library, offline
grammar/spelling cleanup + a Grammarly-style review panel (Harper), an **offline AI chat + GPU local LLM**,
and a premium Linear/Raycast-style UI. Primary user is a **clinician**; the beachhead is clinical workflows.

- **Repo:** `github.com/cspergel/dotflow` (`origin`). `upstream` = `github.com/cjpais/Handy`.
- **Local path:** `~/Documents/Coding Projects/dotflow`. **Data dir:** `%APPDATA%/com.dotflow.app/`.
- **Design docs:** `docs/dotflow-design/` (`ROADMAP.md`, this file) + `docs/plans/`.

## Branch model (two shipping versions)

- `main` — CPU-only dictation version (untouched, works well). **CPU standalone not made official yet.**
- `feat/review-enhancements` — CPU-plus (main + overlay-wedge fix + medical dict pack; NO chat).
- **`feat/ai-chat-gpu`** ← **current** — CPU-plus + AI chat + GPU + all the 2026-07-09 work, built on top.
  Chat/LLM is behind the `local-llm` cargo feature (off by default) = doubly isolated.

## The running GPU app (how the user launches + tests)

- Self-contained GPU app lives at **`C:\Users\drcra\DotFlow-GPU\`** + Desktop shortcut **"DotFlow (GPU)"**
  (runs `DotFlow-GPU.vbs`, which prepends CUDA Toolkit v13.3 `bin\x64` to PATH so it finds cublas/cudart).
- Each build is manually **swapped** into that folder (copy `dotflow.exe` + `*.dll`). Latest swap: **2026-07-09
  ~23:31** (q8-KV / 32k build). Card = RTX **5080** (16 GB). Single-instance: close any running copy first.
- **Runtime files that MUST sit next to the exe** (gitignored, NOT vendored — fetch at setup):
  `pdfium.dll` (7 MB, bblanchon/pdfium-binaries), `text-detection.rten` (2.4 MB) + `text-recognition.rten`
  (9.3 MB) (ocrs models, `ocrs-models.s3-accelerate.amazonaws.com`), plus the CUDA DLLs from the build. NOTE:
  the agent is BLOCKED from downloading binaries — the USER runs a PowerShell `iwr` (no `!` prefix — that's
  the PowerShell not-operator and breaks it).

## ⚠️ Building the GPU app — read before you build (hard-won)

```bash
export PATH="$HOME/.cargo/bin:$HOME/.bun/bin:/c/Program Files/NVIDIA GPU Computing Toolkit/CUDA/v13.3/bin:/c/Program Files/NVIDIA GPU Computing Toolkit/CUDA/v13.3/bin/x64:$PATH"
export CUDA_PATH='C:\Program Files\NVIDIA GPU Computing Toolkit\CUDA\v13.3'
export CUDA_PATH_V13_3='C:\Program Files\NVIDIA GPU Computing Toolkit\CUDA\v13.3'   # backslashes! MSBuild reads this
export CUDACXX="C:/Program Files/NVIDIA GPU Computing Toolkit/CUDA/v13.3/bin/nvcc.exe"
export LIBCLANG_PATH="C:/Users/drcra/anaconda3/Lib/site-packages/clang/native"
export CARGO_TARGET_DIR="C:/dtfb"
cargo clean -p dotflow --release --manifest-path src-tauri/Cargo.toml   # forces fresh frontend re-embed
bun run tauri build --no-bundle --features local-llm-cuda              # capture exit code; DON'T mask with | tail
# then swap: taskkill //F //IM dotflow.exe; cp dotflow.exe + *.dll -> C:/Users/drcra/DotFlow-GPU/
```

- **NEVER `cargo clean -p llama-cpp-sys-2`.** It normally rides a cached CMake CUDA configure. Cleaning it
  forces a from-scratch configure that needs `nvcc` on PATH + `CUDA_PATH_V13_3` (both above) or it fails with
  `The CUDA Toolkit directory '' does not exist` / CUDA-compiler-id errors. If you DID clean it and it now
  fails `MSB1009: install.vcxproj does not exist`, **wipe `C:/dtfb/release/build/llama-cpp-sys-2-*`** and
  rebuild with the full env so it does a clean configure. (A `cargo add` that bumps a shared transitive dep,
  e.g. `typenum`, can also force this rebuild — pin deps to avoid it.)
- A clean CUDA recompile grows the exe ~124 MB → ~220 MB (kernels for all arches) — harmless; slim later via
  `CMAKE_CUDA_ARCHITECTURES` if wanted.
- Verify after build: `bunx tsc --noEmit` + `bun run lint` clean; Rust test suite ~218 passing.

## What shipped this session (2026-07-09) — all live + pushed

**Dictionaries & cleanup:** Dictionaries is its own sidebar tab; in-app **"My words"** custom pack
(`custom.txt`); medical dict pack (acceptance-only, never auto-applies). Command surface (P1) in the review
overlay: a **"type or say what to do…"** input (`ai_transform_custom`) + pinned chips, plus two clinical
actions **"Plain language"** and **"Extract"** (meds/problems/allergies). "Before" box collapses after a
transform; footer pinned. Transforms fixed for reasoning models (`/no_think` + input-scaled budget) + a
**per-task model picker** (Gemma for transforms while Qwythos chats) + a **reasoning toggle** (chat / quick-chat
/ review).

**Chat:** markdown rendering (tables/lists/code), live context gauge, recent-chats slide-out + expand-to-chat
handoff, **streaming dictation into the box**, auto-grow composers, chat cutoff fixed (answer cap 8192).

**Documents:** attach a **text PDF** → summarize/ask (auto-expands context). **Scanned-PDF OCR** (pdfium
rasterize + ocrs, CPU) — **works great: 53k chars in ~20s** on the user's 29-page chart. Page-tolerant.

**Long context:** **q8 KV cache + flash attention** → **32k context fits in 16 GB** (was OOM-crashing at 32k
with fp16 KV). Caps at 32k everywhere.

**Model:** Qwythos-9B (Claude-Mythos, qwen35 arch, 1M ctx) = chat model; Gemma for transforms. **Qwythos HAS
vision** via its `mmproj-*.gguf` (= base Qwen3.5-9B multimodal) — not yet wired.

## PENDING USER TESTS (verify first thing)

1. **OCR + 32k on a real chart:** set context to 32k, OCR `Clinicals_and_3008.pdf`, summarize + ask pointed
   follow-ups. Confirm **no OOM crash** and good recall. **Runtime unknown:** flash-attn must support Qwen3.5
   head dims at 32k — if the model *fails to load*, fall back (e.g. `V=q5_1`, or lower the cap).
2. **OCR quality** on real clinical faxes (the big unknown). Decides if CPU-OCR is "done" or we escalate to a
   GPU/ONNX or VLM OCR path.
3. Sanity-check the rest of the session's features in real use.

## Upcoming work (prioritized, discussed with user)

- **Vision via `llama-server` sidecar** — the big foundational investment. Qwythos+mmproj behind an
  OpenAI-compatible vision API (DotFlow already speaks that shape) routes around the `llama-cpp-2` mtmd blocker.
  **One investment unlocks: VLM-OCR (hard/handwritten docs), image/screenshot chat, AND screen perception.**
- **Novel screen-use ideas** (user excited): **ambient clinical safety-net** (real-time flag wrong dose /
  allergy / wrong-chart-open — only a *local* model can watch a PHI screen continuously), **Citrix/VDI-proof
  universal OCR grab** (grab uncopyable text anywhere), ethically-local Rewind timeline, screen-context macros,
  auto-redaction for screenshare. Lean vision toward *perception/alerting*, not pixel-precise clicking (local
  9B VLM weak at coordinate grounding → DOM beats vision for known-app control).
- **Drug-name dictation** (losartan issue): medical DICT pack does NOT affect STT — only proofreading. For
  dictation accuracy use **Custom Words** (STT fuzzy-boost). Offered but deferred: wire medical-pack terms into
  the STT booster (opt-in; over-correction risk).
- **EMR agent** (ROADMAP §Long-term vision) — GATED on user recon: hosted-vs-local + API-vs-DOM. DOM/adapter
  beats vision for the known web EMR; record-to-teach via `playwright codegen`.
- **P2 named actions** (Translate, Tone, Expand, Reply) — cheap prompts behind the command surface.
- **Scanned-PDF OCR GPU upgrade** (if CPU too slow / quality low): PaddleOCR-ONNX via the shipped onnxruntime
  + DirectML, OR the VLM path above.

## Open decisions

- **KV quant:** currently q8_0 K+V (negligible loss, keep for clinical recall). Middle-ground = asymmetric
  `K=q8/V=q5_1` (~7.25 bpw) — but VERIFY q5_1 loads with flash-attn on the 5080 before relying on it. Don't go
  to q4 for chart Q&A. Two-line change in `local_llm.rs`.
- **Merge to main / make CPU build official** — the branch is large + solid; decide when to stabilize.
- **Bundle the runtime binaries** (pdfium.dll + ocrs models) into the installer vs. fetch-at-setup.

## Key files (2026-07-09 additions)

- OCR/PDF: `src-tauri/src/dotflow/pdf_render.rs`, `dotflow/ocr.rs`, `commands/document.rs` (read_pdf_text,
  ocr_pdf); attach + OCR button in `src/components/chat/ChatView.tsx`.
- LLM: `dotflow/local_llm.rs` (q8 KV + flash), `commands/chat.rs` (n_ctx clamp), `commands/ai.rs`
  (ai_transform / ai_transform_custom, `/no_think`, budget), per-task models in `commands/llm.rs` + settings
  `task_models` / `model_for_task`.
- Chat UI: `src/components/chat/{ChatView,QuickChat,ChatMarkdown,useChatDictation,chatStore}.*`.
- Review: `src/overlay/review/ReviewOverlay.tsx` (command surface + chips + reasoning toggle).
- Dictionaries: `dotflow/dictionary_packs.rs`, `commands/dictionary.rs`, `src/components/settings/dictionaries/`.
