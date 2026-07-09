# Offline AI Chat — design (v1)

> Branch: `feat/review-enhancements`. A personal-first, fully-offline chat feature: ask questions, paste text,
> get suggestions, riff with a local model. Reuses the existing `local_llm` inference + model catalog.

## Goal

An in-app **offline AI chat** — type or paste, ask questions, get writing suggestions, riff — running entirely
on the local GGUF model. No network. Personal power-user feature; the same surface is model-agnostic so it
also works with clean/commercial models.

## Confirmed decisions (brainstormed with user)

1. **Streaming** — tokens render live as generated (not wait-then-show).
2. **Two entry points, one reusable `ChatView`:**
   - **Full "AI Chat" sidebar section** in the main window.
   - **Quick slide-out/popover** from the **condensed (mini/bar) view** for fast questions without opening the
     full window.
3. **Models — ship suggested + import any:** a **model dropdown** listing installed catalog models
   (Gemma 4 / Qwen — license-friendly, shipped/downloadable) **plus** the ability to **import any local GGUF**
   (bring-your-own, e.g. Qwythos) via a file picker. Personal use = any model; commercial = default to the
   clean suggested ones.
4. **Multi-turn** — conversation history is sent each turn (bounded by the model context window) so it
   remembers the thread.
5. **Feature-gated** — only active in `local-llm` builds (the user's GPU build ✓); a clear "pick/import a
   model" empty state otherwise.

## Backend (Rust)

- **Generalize `local_llm` chat to multi-turn + streaming.** Today `generate_chat(model, system, user, max)` is
  single-turn + blocking. Add:
  - a prompt builder over an **ordered message list** `&[(Role, String)]` (reuse the existing chat-template /
    Gemma / ChatML logic in `build_chat_prompt`, generalized to N messages);
  - `generate_chat_stream(model_path, messages, max_tokens, on_token)` — the existing token-by-token
    `run_generation` loop, calling `on_token(piece)` per detokenized piece. Same panic-safety (`catch_unwind`).
- **Command `chat_stream(app, messages, model_path?)`** (feature-gated): spawns generation on a blocking
  thread and emits Tauri events to the frontend: `chat-token {id, text}` per piece, `chat-done {id}`,
  `chat-error {id, message}`. `id` correlates a turn so a stale stream can be ignored. A `chat_cancel(id)`
  sets a flag the generation loop checks (cooperative stop).
- **`list_chat_models()` / `import_chat_model(path)`** — reuse the installed-model list from
  `commands/llm.rs`; import validates the file is a readable `.gguf` and returns it as a selectable entry
  (stored as the chat model path; does not disturb the review-overlay `local_llm_model_path` unless the user
  chooses "use everywhere").

## Frontend (React)

- **`ChatView` (reusable):** message list (user/assistant bubbles), input (Enter=send, Shift+Enter=newline),
  **New chat**, a **model dropdown** (installed + Import…), streaming render (append `chat-token` to the live
  assistant message; stop on `chat-done`). Copy-message button. Conversation state in a small store.
- **Sidebar section "AI Chat"** (add to `SECTIONS_CONFIG` in `Sidebar.tsx`) → renders `ChatView` in the main
  pane.
- **Condensed slide-out:** a chat affordance in the mini/bar view (`App.tsx` view modes) that slides out a
  compact `ChatView` for quick questions, then collapses.

## Out of scope v1 (fast-follows)

- Voice control / dictate-a-command (P3 territory — explicitly later).
- Conversation persistence/history across restarts; per-chat editable system prompt; file/RAG attach;
  regenerate/edit-message. Keep v1 to a working streaming chat.

## Tests + verification

- **Real unit tests** on the pure logic: the multi-turn prompt builder (given an ordered `[system,user,
  assistant,user]` convo, the rendered prompt preserves role order + correct template markers + ends at the
  assistant tag; a fail case with an empty/blank message), and context-window truncation (oldest turns
  dropped first, newest kept). These don't need a model.
- **Live verification** (the honest proof for streaming + UI): run the GPU build, open AI Chat, send a
  multi-turn conversation, confirm tokens stream in and context is remembered; switch models; try the
  condensed slide-out. `cargo test --lib` stays green (existing 188 + new).
