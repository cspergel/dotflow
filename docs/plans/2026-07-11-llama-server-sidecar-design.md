# DotFlow — llama-server Sidecar Design (2026-07-11)

Branch: `feat/ai-chat-gpu`

## Why

The current local LLM runs **in-process** via `llama-cpp-2`. Any CUDA fault (OOM, flash-attn abort) is an
**uncatchable** C-level crash that takes down the whole app — confirmed repeatedly this session. That same
in-process design also caps us at a **16k context** (bigger fp16 KV OOMs on the 16GB RTX 5080), which forces
heavy map/reduce compression that drops content from 100+ page clinical charts.

Running **llama.cpp's `llama-server` as a subprocess** fixes all three at once:

1. **Crash-isolation** — a CUDA fault fails one HTTP request; the app survives and restarts the server.
2. **Safe 32k+ context** — the real fix for completeness on big charts (bigger chunks → fewer lossy reductions).
3. **Vision** — Qwythos ships an mmproj; `llama-server --mmproj` serves it over the same API.

"One investment, three wins."

## Decisions (settled with the user)

1. **Model strategy: ONE model (Qwythos) for everything** — a single `llama-server` instance at 32k handles
   extraction, synthesis, chat, and vision. Simplest topology; fits 16GB (Qwythos Q4 ~5.6GB + q8 KV @32k ~5GB
   + mmproj ~1GB ≈ 11–12GB). Crash-isolation removes the reason we split extraction to Gemma. Map/reduce stays,
   but with far larger chunks. (Gemma-for-extraction lives on only in the in-process fallback.)
2. **Migration: sidecar-preferred + in-process fallback** — route to the sidecar when present & healthy; fall
   back to today's in-process 16k path otherwise. With no binary present, behavior is byte-for-byte identical to
   today. Purely additive, A/B-able.
3. **Binary: user downloads the official prebuilt `llama-server` CUDA build** (self-contained: `llama-server.exe`
   + its own `ggml-cuda` DLLs). Lives in a `llama-server/` folder next to the app; DotFlow auto-detects it (like
   Tesseract). Isolated from the in-process ggml DLLs (avoids the ggml-base.dll collision from the handoff).
4. **Fallback must be VISIBLE (user requirement)** — never silently degrade.

## Architecture

- **`SidecarManager` (Rust, Tauri state):** auto-detects `llama-server.exe`, spawns it on `127.0.0.1:<free-port>`
  with Qwythos + mmproj, `--ctx-size 32768`, flash-attention + q8 KV; health-checks; monitors the child;
  restarts-once-with-backoff on crash; kills it on app exit.
- **LLM router (`dotflow::llm`):** single entry point in front of both backends. Checks sidecar health
  **per request**; dispatches to the sidecar (HTTP) when healthy, else to in-process `local_llm`. All callers
  (`run_transform`, `chat_stream`, summarize map/reduce) go through it.
- **HTTP client (`reqwest`):** talks to the sidecar's OpenAI-compatible `/v1/chat/completions` (streaming for
  chat, non-streaming for extract/synth).
- **`local_llm` (unchanged):** the in-process implementation behind the router; keeps `model_for_task` split +
  16k cap on the fallback path.

## Backend visibility (the fallback-transparency requirement)

- Event `llm-backend-status { backend, ctx, reason }` drives a **chat-header badge**: `⚡ 32k · GPU sidecar`
  (green) vs `⚠ 16k · in-process (fallback)` (amber, tooltip explains the difference).
- **On any fallback** — binary missing, won't start, health-check timeout, or the sidecar dies mid-session — a
  **toast** fires: *"AI sidecar stopped — using 16k in-process mode. Long-document summaries may be less
  complete."* You always know which engine answered.

## Lifecycle

- **Spawn:** lazily, **when the user enters the chat section** (not at app launch — many launches are
  dictation-only, and Qwythos load costs seconds + VRAM). "Starting the AI engine…" status if a send beats the load.
- **Health:** poll `GET /health` until 200 (server ready), ~60s ceiling. Until healthy → in-process + amber badge.
- **Port:** auto-selected free `127.0.0.1` port, local-only, no API key.
- **Shutdown:** kill child on app exit (exit hook + Drop guard) so no orphaned `llama-server.exe` holds VRAM;
  same on model change.
- **Crash:** monitor child; unexpected exit → next request falls back (+ toast) → **one** restart with backoff
  (no infinite loop). If restart fails, stay on in-process until re-entering chat.
- **Idle shutdown:** skipped for v1 (YAGNI).

## Streaming & requests

- **Chat (streaming):** POST `stream:true` → SSE (`data: {…delta.content…}` … `data: [DONE]`). Rust reads the
  async byte stream, parses SSE, emits the existing `chat-token` / `chat-done` / `chat-error` events → **zero
  frontend change** for chat rendering.
- **Summarize extract/synth (non-streaming):** POST `stream:false` → `choices[0].message.content` → drops into
  the existing map/reduce (sidecar impl of `generate_chat`).
- **Message mapping:** `ChatTurn{role,content}` → OpenAI `messages`. `n_ctx`/max-tokens/temp/`/no_think` (or
  `enable_thinking:false`) are request params. `<think>` stripping unchanged.
- **Cancellation:** `chat_cancel(id)` aborts the `reqwest` request (drops the connection → server stops).
- **Context:** sidecar requests pass **32768**; the chat UI gains a 32k option enabled only when the sidecar is
  the active backend. In-process stays 16k.
- **Dependency:** add `reqwest` (async + stream + rustls) — likely already transitive via the updater plugin.

## Vision (built after the text core)

- Spawn with `--mmproj <mmproj-Qwythos…F16.gguf>` when present. Images ride the same endpoint via
  `content: [{type:"text",…},{type:"image_url", image_url:{url:"data:image/png;base64,…"}}]`.
- **v1 scope:** an "attach image" button in chat → base64 → Qwythos vision → streamed answer (wound photo, ECG,
  form, screenshot). Requires the sidecar; on fallback, show "Image understanding needs the AI sidecar."
- **NOT for bulk OCR** — Tesseract stays the OCR engine (faster, higher-fidelity on dense text). Defer
  ambient/screen-capture ideas (YAGNI).

## Testing

- **Automated (teeth):** SSE parser (canned body → exact token sequence, stops on `[DONE]`); router selection
  (mocked health → dispatches sidecar-when-healthy, in-process-when-not; canary flips when health=false);
  free-port probe; OpenAI message mapping; health-state-machine transitions.
- **Manual/on-device:** real spawn → chat streams; 32k summarize; **kill server mid-request → fallback + toast**;
  vision.

## Phased build order

Each phase builds → swaps → tests; everything falls back to in-process, so nothing breaks mid-migration.

- **Phase 0 (user, parallel):** download prebuilt `llama-server` CUDA build; confirm Qwythos GGUF + mmproj paths.
  **CRITICAL: verify the downloaded `llama-server` actually loads Qwythos** (gated-delta-net / qwen3-next arch)
  by running it manually once — if the prebuilt release's llama.cpp is too old, it won't load Qwythos and we'd
  need a newer build. De-risks the whole effort before we wire routing.
- **Phase 1:** `SidecarManager` — detect, spawn, health-check, status badge + fallback toast, clean shutdown.
  Prove the server comes up and the badge flips green. No routing yet. (Buildable now; needs the binary to test.)
- **Phase 2:** router + **chat** streaming over SSE → chat at 32k, crash-safe.
- **Phase 3:** **summarize** on the sidecar (bigger chunks @32k) — the completeness payoff for clinical charts.
- **Phase 4:** transforms on the sidecar.
- **Phase 5:** **vision** (`--mmproj` + image attach).

## Risks / assumptions to verify

- **Prebuilt `llama-server` must support Qwythos's architecture + mmproj** — verify in Phase 0 (biggest risk).
- **VRAM at 32k** — Qwythos Q4 + q8 KV + mmproj should fit ~12GB/16GB; confirm on-device, drop ctx if needed.
- **No orphaned processes** — Drop guard + exit hook must reliably kill the child on every exit path.
- **Per-request health check race** — a server dying between the health check and the request → treat the HTTP
  error as a fallback trigger for that request too (belt-and-suspenders).
- **`reqwest` ggml/DLL isolation** — the sidecar's CUDA DLLs live in its own folder; the app never loads them.
