//! Local LLM text generation for DotFlow (feature-gated behind `local-llm`).
//!
//! This is the effectful, offline text-generation shell that DotFlow's selection→review overlay
//! will call to rewrite/clean up a highlighted passage using a small local instruct model. It wraps
//! the `llama-cpp-2` crate (v0.1.151) — a thin, *unsafe* binding over llama.cpp — behind a single
//! total-ish function that never panics into the caller: any llama.cpp panic is caught and mapped to
//! `Err(String)` so a bad model / OOM / assert cannot take down the Tauri app.
//!
//! Everything here is CPU-only (`n_gpu_layers = 0`) so it coexists with transcribe-cpp's GPU whisper
//! backend without fighting over the GPU. The llama backend is initialized exactly once, process-wide,
//! via `OnceLock` — llama.cpp's `llama_backend_init` must not run twice.
//!
//! Loading a GGUF from disk costs ~1-2 GB of I/O + parsing per call, so the most-recently-used model is
//! cached process-wide in a `Mutex<Option<CachedModel>>` keyed by path (+ load params). A generate call
//! reuses the cached model when the path matches and only reloads when the path changes. `LlamaModel` is
//! `Send + Sync` (llama-cpp-2 marks it so), which makes caching it in a `static` sound; a fresh
//! `LlamaContext` is still created per call (contexts are cheap and hold per-generation KV state — never
//! reuse one). Because a context borrows `&model` out of the guard, the whole generation runs while the
//! cache lock is held, so concurrent transforms serialize — fine, since DotFlow rewrites one selection
//! at a time.
//!
//! API shape used (llama-cpp-2 0.1.151):
//!   LlamaBackend::init() -> once, stored in OnceLock
//!   LlamaModelParams::default().with_n_gpu_layers(0)
//!   LlamaModel::load_from_file(&backend, path, &params)
//!   LlamaContextParams::default().with_n_ctx(Some(NonZeroU32))
//!   model.new_context(&backend, ctx_params)
//!   model.str_to_token(prompt, AddBos::Always)   // AddBos respects the model's tokenizer config
//!   LlamaBatch::new(cap, 1); batch.add(tok, pos, &[0], last_logits_bool)
//!   ctx.decode(&mut batch)
//!   LlamaSampler::greedy(); sampler.sample(&ctx, idx); sampler.accept(tok)
//!   model.is_eog_token(tok) to stop; model.token_to_str(tok, Special::Plaintext) to detokenize

use std::num::NonZeroU32;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use llama_cpp_2::context::params::LlamaContextParams;
use llama_cpp_2::llama_backend::LlamaBackend;
use llama_cpp_2::llama_batch::LlamaBatch;
use llama_cpp_2::model::params::LlamaModelParams;
use llama_cpp_2::model::{AddBos, LlamaChatMessage, LlamaModel, Special};
use llama_cpp_2::sampling::LlamaSampler;

/// Process-wide llama.cpp backend. `llama_backend_init` is not safe to call twice, so it is guarded
/// by a `OnceLock` and shared across every `generate()` call.
static BACKEND: OnceLock<LlamaBackend> = OnceLock::new();

fn backend() -> Result<&'static LlamaBackend, String> {
    // `get_or_init` cannot return a Result, so init into an Option and surface the error after.
    if BACKEND.get().is_none() {
        match LlamaBackend::init() {
            Ok(b) => {
                // Ignore the Err(value) case: it only happens if another thread won the race, which
                // is exactly the outcome we want (a single shared backend).
                let _ = BACKEND.set(b);
            }
            Err(e) => return Err(format!("failed to init llama backend: {e}")),
        }
    }
    BACKEND
        .get()
        .ok_or_else(|| "llama backend unavailable after init".to_string())
}

/// How many model layers to offload to the GPU when loading a GGUF.
///
/// The default (CPU) build keeps every layer on the CPU (`0`) so it doesn't contend with the GPU
/// whisper backend. The `cuda` feature (via `local-llm-cuda`) offloads all layers to the GPU — `999`
/// is llama.cpp's idiomatic "all layers" sentinel (it's clamped to the model's real layer count).
#[cfg(feature = "cuda")]
const N_GPU_LAYERS: u32 = 999;
#[cfg(not(feature = "cuda"))]
const N_GPU_LAYERS: u32 = 0;

/// A loaded GGUF plus the inputs that determined how it was loaded, so we can tell whether a cached
/// model still matches the current request.
struct CachedModel {
    path: PathBuf,
    n_gpu_layers: u32,
    model: LlamaModel,
}

/// Most-recently-used loaded model, shared across every `generate*` call. Holds at most one model; a
/// request for a different path drops the previous one and loads the new one. `Mutex::new` is const, so
/// this needs no lazy initialization.
static MODEL_CACHE: Mutex<Option<CachedModel>> = Mutex::new(None);

/// Run `f` with a loaded model for `model_path`, reusing the process-wide cached model when the path and
/// load params match, otherwise loading it (and dropping the previously cached one) first.
///
/// The closure runs while the cache lock is held because the `LlamaContext` it creates borrows `&model`
/// out of the guard; this serializes concurrent generations, which is acceptable here. The lock is
/// recovered on poison (`into_inner`): the cached model is only ever read during generation, so a panic
/// mid-generation leaves it in a consistent state and later calls can keep using it — preserving the
/// panic-safety guarantee of [`generate`]/[`generate_chat`].
fn with_cached_model<T>(
    model_path: &Path,
    f: impl FnOnce(&LlamaBackend, &LlamaModel) -> Result<T, String>,
) -> Result<T, String> {
    if !model_path.exists() {
        return Err(format!(
            "model file does not exist: {}",
            model_path.display()
        ));
    }

    let backend = backend()?;
    let mut guard = MODEL_CACHE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());

    let hit = matches!(
        guard.as_ref(),
        Some(c) if c.path == model_path && c.n_gpu_layers == N_GPU_LAYERS
    );
    if !hit {
        // Drop the previously cached model before loading so we never hold two (~1-2 GB each) at once.
        *guard = None;
        let model_params = LlamaModelParams::default().with_n_gpu_layers(N_GPU_LAYERS);
        let model = LlamaModel::load_from_file(backend, model_path, &model_params)
            .map_err(|e| format!("failed to load model: {e}"))?;
        *guard = Some(CachedModel {
            path: model_path.to_path_buf(),
            n_gpu_layers: N_GPU_LAYERS,
            model,
        });
    }

    let cached = guard.as_ref().expect("cache populated above");
    f(backend, &cached.model)
}

/// Drop the process-wide cached model, freeing its ~1-2 GB immediately.
///
/// Called when the active model's file is deleted so we don't keep a now-orphaned model resident (and,
/// worse, keep serving generations from a model whose backing file is gone). The lock is recovered on
/// poison (`into_inner`) for the same reason [`with_cached_model`] does: the cache only holds a loaded
/// model, so clearing it is always safe regardless of a prior panic. A subsequent `generate*` call sees
/// an empty cache and reloads from disk (or errors cleanly if the file is missing).
pub fn evict_cache() {
    let mut guard = MODEL_CACHE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    *guard = None;
}

/// Generate text from a local GGUF instruct model, fully offline on CPU.
///
/// `prompt` is fed verbatim (the caller is responsible for any chat-template wrapping, e.g. ChatML for
/// Qwen). Generation is greedy (deterministic) and stops at the model's end-of-generation token or after
/// `max_new_tokens` new tokens, whichever comes first. Returns the decoded continuation only (the prompt
/// is not echoed back).
///
/// Any panic from the underlying (unsafe) llama.cpp binding is caught and returned as `Err(String)` so
/// it can never unwind into — and crash — the host application.
pub fn generate(model_path: &Path, prompt: &str, max_new_tokens: usize) -> Result<String, String> {
    // `LlamaContext`/`LlamaModel` hold raw pointers and are not `UnwindSafe`; we intentionally discard
    // all of that state on panic (Drop runs during unwind), so asserting unwind-safety is sound here.
    let result = catch_unwind(AssertUnwindSafe(|| {
        generate_inner(model_path, prompt, max_new_tokens)
    }));
    match result {
        Ok(inner) => inner,
        Err(payload) => {
            let msg = if let Some(s) = payload.downcast_ref::<&str>() {
                (*s).to_string()
            } else if let Some(s) = payload.downcast_ref::<String>() {
                s.clone()
            } else {
                "unknown panic".to_string()
            };
            Err(format!("llama.cpp panicked during generation: {msg}"))
        }
    }
}

/// Chat-templated generation: wrap `system` + `user` in the model's chat template (built-in when the
/// GGUF ships one, else a ChatML fallback) and generate the assistant's reply. This is what DotFlow's
/// "AI transform" chips (Rewrite / Formal / Summarize) call — each supplies a per-action SYSTEM prompt
/// and the selected text as the USER turn. Same greedy, offline, CPU-only, panic-safe guarantees as
/// [`generate`].
pub fn generate_chat(
    model_path: &Path,
    system: &str,
    user: &str,
    max_new_tokens: usize,
) -> Result<String, String> {
    let result = catch_unwind(AssertUnwindSafe(|| {
        generate_chat_inner(model_path, system, user, max_new_tokens)
    }));
    match result {
        Ok(inner) => inner,
        Err(payload) => {
            let msg = if let Some(s) = payload.downcast_ref::<&str>() {
                (*s).to_string()
            } else if let Some(s) = payload.downcast_ref::<String>() {
                s.clone()
            } else {
                "unknown panic".to_string()
            };
            Err(format!("llama.cpp panicked during generation: {msg}"))
        }
    }
}

/// Streaming, multi-turn chat generation — the engine behind DotFlow's offline chat panel. Builds the prompt
/// from the ordered `messages` (model chat template, else Gemma/ChatML fallback), then greedily decodes,
/// invoking `on_token` with each decoded piece as it is produced so the UI can render tokens live.
/// `should_cancel` is polled each step for a cooperative early stop (the chat "Stop" button). Returns the
/// full **cleaned** reply (the streamed pieces are raw; the caller should treat this return as authoritative
/// on completion). Same panic-safety as [`generate_chat`]: any llama.cpp panic is caught and returned as
/// `Err`, never unwound into the host.
pub fn generate_chat_stream(
    model_path: &Path,
    messages: &[ChatTurn],
    max_new_tokens: usize,
    n_ctx: u32,
    mut on_token: impl FnMut(&str),
    should_cancel: &dyn Fn() -> bool,
) -> Result<String, String> {
    let result = catch_unwind(AssertUnwindSafe(|| {
        with_cached_model(model_path, |backend, model| {
            let (prompt, add_bos) = build_chat_prompt_multi(model, messages);
            run_generation(
                backend,
                model,
                &prompt,
                add_bos,
                max_new_tokens,
                n_ctx,
                &mut on_token,
                should_cancel,
            )
        })
        .map(|s| clean_chat_output(&s))
    }));
    match result {
        Ok(inner) => inner,
        Err(payload) => {
            let msg = if let Some(s) = payload.downcast_ref::<&str>() {
                (*s).to_string()
            } else if let Some(s) = payload.downcast_ref::<String>() {
                s.clone()
            } else {
                "unknown panic".to_string()
            };
            Err(format!("llama.cpp panicked during generation: {msg}"))
        }
    }
}

/// Build the prompt string for a system+user chat turn. Prefers the template baked into the GGUF (so
/// Qwen, Gemma, Llama, … each get their own correct markers); falls back to ChatML — which suits the
/// Qwen2.5 test model — when the model ships no template or applying it fails. Returns the prompt plus
/// the `AddBos` policy: the built-in template owns any BOS (so `Never`), while the bare ChatML fallback
/// defers to the tokenizer config (`Always`), matching [`generate`].
/// True if `marker` is a single control/special token in this model's vocab (e.g. Gemma's
/// `<start_of_turn>`). Used to pick the right chat template when the model's baked-in template can't be
/// rendered by llama.cpp's built-in applier.
fn has_control_token(model: &LlamaModel, marker: &str) -> bool {
    model
        .str_to_token(marker, AddBos::Never)
        .map(|t| t.len() == 1)
        .unwrap_or(false)
}

/// A chat role in a multi-turn conversation. The offline chat feature sends an ordered list of these; the
/// single-turn AI-transform path is just `[System, User]`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    System,
    User,
    Assistant,
}

impl Role {
    /// The role string used by ChatML markers and llama.cpp's chat-template applier.
    fn as_str(self) -> &'static str {
        match self {
            Role::System => "system",
            Role::User => "user",
            Role::Assistant => "assistant",
        }
    }
}

/// One turn in a chat conversation.
#[derive(Clone, Debug)]
pub struct ChatTurn {
    pub role: Role,
    pub content: String,
}

/// Render a conversation in **ChatML** (Qwen-style), ending at the assistant tag so the model completes the
/// reply. `parse_special=true` in `str_to_token` turns these markers into real special tokens. Preserves the
/// exact turn order — this is what lets a multi-turn chat "remember" earlier messages.
fn format_chatml(messages: &[ChatTurn]) -> String {
    let mut out = String::new();
    for m in messages {
        out.push_str(&format!(
            "<|im_start|>{}\n{}<|im_end|>\n",
            m.role.as_str(),
            m.content
        ));
    }
    out.push_str("<|im_start|>assistant\n");
    out
}

/// Render a conversation for **Gemma**. Gemma has no system role and calls the assistant "model", and it does
/// NOT understand ChatML (it echoes `<|im_end|>` as literal text and never stops). So use Gemma's own turn
/// markers; fold any system content into the first user turn. Ends at the `model` tag for completion.
fn format_gemma(messages: &[ChatTurn]) -> String {
    let mut out = String::new();
    let mut system_prefix = String::new();
    let mut folded = false;
    for m in messages {
        match m.role {
            Role::System => {
                if !m.content.trim().is_empty() {
                    if !system_prefix.is_empty() {
                        system_prefix.push_str("\n\n");
                    }
                    system_prefix.push_str(&m.content);
                }
            }
            Role::User => {
                let content = if !folded && !system_prefix.is_empty() {
                    folded = true;
                    format!("{system_prefix}\n\n{}", m.content)
                } else {
                    m.content.clone()
                };
                out.push_str(&format!("<start_of_turn>user\n{content}<end_of_turn>\n"));
            }
            Role::Assistant => {
                out.push_str(&format!(
                    "<start_of_turn>model\n{}<end_of_turn>\n",
                    m.content
                ));
            }
        }
    }
    out.push_str("<start_of_turn>model\n");
    out
}

/// Build the prompt for a multi-turn conversation. Prefers the template baked into the GGUF (so Qwen, Gemma,
/// Llama, … each get their own correct markers); falls back to Gemma markers (if the model has Gemma's control
/// tokens) else ChatML. Returns the prompt plus the `AddBos` policy: the built-in template owns any BOS (so
/// `Never`), while the bare fallbacks defer to the tokenizer config (`Always`).
fn build_chat_prompt_multi(model: &LlamaModel, messages: &[ChatTurn]) -> (String, AddBos) {
    let llama_msgs: Vec<LlamaChatMessage> = messages
        .iter()
        .filter_map(|m| LlamaChatMessage::new(m.role.as_str().to_string(), m.content.clone()).ok())
        .collect();
    // add_ass=true leaves the prompt hanging at the assistant tag so the model completes the reply.
    if llama_msgs.len() == messages.len() {
        if let Ok(tmpl) = model.chat_template(None) {
            if let Ok(rendered) = model.apply_chat_template(&tmpl, &llama_msgs, true) {
                return (rendered, AddBos::Never);
            }
        }
    }
    if has_control_token(model, "<start_of_turn>") {
        return (format_gemma(messages), AddBos::Always);
    }
    (format_chatml(messages), AddBos::Always)
}

/// Single-turn convenience over [`build_chat_prompt_multi`] for the AI-transform path (`[System, User]`).
fn build_chat_prompt(model: &LlamaModel, system: &str, user: &str) -> (String, AddBos) {
    let messages = [
        ChatTurn {
            role: Role::System,
            content: system.to_string(),
        },
        ChatTurn {
            role: Role::User,
            content: user.to_string(),
        },
    ];
    build_chat_prompt_multi(model, &messages)
}

/// Trim trailing chat/control markers a model may emit as literal text (e.g. a mismatched-template
/// `<|im_end|>`) so the returned result is clean. Cuts from the first such marker onward.
fn clean_chat_output(s: &str) -> String {
    let mut end = s.len();
    for marker in [
        "<|im_end|>",
        // Partial variant: some models emit `|im_end|>` when the leading `<` tokenizes into the prior
        // piece, so the full `<|im_end|>` never appears as a contiguous substring (seen with Gemma).
        "|im_end|>",
        "<end_of_turn>",
        "<|im_start|>",
        "|im_start|>",
        "<eos>",
        "</s>",
        "<|endoftext|>",
    ] {
        if let Some(pos) = s.find(marker) {
            end = end.min(pos);
        }
    }
    s[..end].trim().to_string()
}

fn generate_chat_inner(
    model_path: &Path,
    system: &str,
    user: &str,
    max_new_tokens: usize,
) -> Result<String, String> {
    with_cached_model(model_path, |backend, model| {
        let (prompt, add_bos) = build_chat_prompt(model, system, user);
        run_generation(
            backend,
            model,
            &prompt,
            add_bos,
            max_new_tokens,
            // 16384 (was 8192) so a longer selection + a reasoning model's <think> pass + the answer all fit
            // before the context cap. Clamped down to the model's trained max inside run_generation.
            16384,
            &mut |_| {},
            &|| false,
        )
    })
    .map(|s| clean_chat_output(&s))
}

fn generate_inner(
    model_path: &Path,
    prompt: &str,
    max_new_tokens: usize,
) -> Result<String, String> {
    with_cached_model(model_path, |backend, model| {
        run_generation(
            backend,
            model,
            prompt,
            AddBos::Always,
            max_new_tokens,
            8192,
            &mut |_| {},
            &|| false,
        )
    })
}

/// Create a context, tokenize `prompt`, and greedily decode up to `max_new_tokens`. Shared by the plain
/// [`generate`], chat, and streaming-chat paths. `add_bos` lets the chat path suppress a BOS the template
/// already emitted. `on_token` receives each newly decoded piece of text as it is produced (for streaming);
/// pass a no-op for the batch paths. `should_cancel` is polled before each decode step so a caller can stop
/// generation early.
fn run_generation(
    backend: &LlamaBackend,
    model: &LlamaModel,
    prompt: &str,
    add_bos: AddBos,
    max_new_tokens: usize,
    requested_n_ctx: u32,
    on_token: &mut dyn FnMut(&str),
    should_cancel: &dyn Fn() -> bool,
) -> Result<String, String> {
    // Context window = what the caller asked for, floored at 512 and capped at what the model was actually
    // trained on (so we never ask for more than the weights support — e.g. a 9B "1M" model still caps at its
    // real trained length). The KV cache scales linearly with this, so the chat UI exposes it as a setting to
    // trade VRAM for a longer memory.
    let n_ctx = requested_n_ctx.clamp(512, model.n_ctx_train().max(512));
    let ctx_params = LlamaContextParams::default().with_n_ctx(Some(
        NonZeroU32::new(n_ctx).unwrap_or(NonZeroU32::new(2048).unwrap()),
    ));
    let mut ctx = model
        .new_context(backend, ctx_params)
        .map_err(|e| format!("failed to create context: {e}"))?;

    // Tokenize. AddBos::Always defers to the model's tokenizer config (llama.cpp only prepends BOS if
    // the model is configured to want one — Qwen2.5, for example, is not), so this is "BOS as the
    // model expects", not an unconditional BOS.
    let tokens = model
        .str_to_token(prompt, add_bos)
        .map_err(|e| format!("failed to tokenize prompt: {e}"))?;
    if tokens.is_empty() {
        return Err("prompt tokenized to zero tokens".to_string());
    }

    let n_ctx_usize = n_ctx as usize;
    if tokens.len() >= n_ctx_usize {
        return Err(format!(
            "prompt ({} tokens) does not fit in context window ({})",
            tokens.len(),
            n_ctx
        ));
    }

    // Cap the new-token budget to what actually fits in the KV cache. The prompt was validated to fit,
    // but greedy decoding appends up to `max_new_tokens` more; without this cap a long prompt + long
    // budget overflows the context mid-decode and surfaces as a confusing "failed to decode token".
    // Leave a small margin below the hard limit for safety.
    const CTX_MARGIN: usize = 8;
    let room_for_new = n_ctx_usize
        .saturating_sub(tokens.len())
        .saturating_sub(CTX_MARGIN);
    if room_for_new == 0 {
        return Err(format!(
            "prompt too long for the model's context window ({} tokens used of {}, no room to generate)",
            tokens.len(),
            n_ctx
        ));
    }
    let effective_max_new = max_new_tokens.min(room_for_new);

    // Batch must hold the whole prompt for the initial decode; size it to the context window.
    let mut batch = LlamaBatch::new(n_ctx_usize, 1);
    let last_prompt_index = (tokens.len() - 1) as i32;
    for (i, token) in tokens.iter().enumerate() {
        // Only the final prompt token needs its logits computed — that's where we sample from.
        let compute_logits = i as i32 == last_prompt_index;
        batch
            .add(*token, i as i32, &[0], compute_logits)
            .map_err(|e| format!("failed to fill prompt batch: {e}"))?;
    }

    ctx.decode(&mut batch)
        .map_err(|e| format!("failed to decode prompt: {e}"))?;

    // Greedy sampling: deterministic and coherent, ideal as a "does it work" proof and for reproducible
    // rewrites. (Swap in a temp/top-k/dist chain later if we want variety.)
    let mut sampler = LlamaSampler::greedy();

    let mut output = String::new();
    let mut n_cur = batch.n_tokens();

    for _ in 0..effective_max_new {
        // Cooperative cancel: a streaming caller (e.g. the chat UI's Stop button) can end generation early.
        if should_cancel() {
            break;
        }

        // Sample from the logits of the last token in the current batch.
        let token = sampler.sample(&ctx, batch.n_tokens() - 1);
        sampler.accept(token);

        if model.is_eog_token(token) {
            break;
        }

        // Detokenize as plaintext so any stray control/special token renders harmlessly rather than as
        // literal marker text.
        match model.token_to_str(token, Special::Plaintext) {
            Ok(piece) => {
                on_token(&piece);
                output.push_str(&piece);
            }
            Err(e) => return Err(format!("failed to detokenize token: {e}")),
        }

        // Feed the sampled token back in as the next single-token batch.
        batch.clear();
        batch
            .add(token, n_cur, &[0], true)
            .map_err(|e| format!("failed to add token to batch: {e}"))?;
        n_cur += 1;

        ctx.decode(&mut batch)
            .map_err(|e| format!("failed to decode token: {e}"))?;
    }

    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn convo() -> Vec<ChatTurn> {
        // A 4-turn conversation: system + two user turns with an assistant reply between them.
        vec![
            ChatTurn {
                role: Role::System,
                content: "You are helpful.".to_string(),
            },
            ChatTurn {
                role: Role::User,
                content: "Hi".to_string(),
            },
            ChatTurn {
                role: Role::Assistant,
                content: "Hello!".to_string(),
            },
            ChatTurn {
                role: Role::User,
                content: "What is 2+2?".to_string(),
            },
        ]
    }

    /// Ordering is the whole point of multi-turn: every turn must appear, in conversation order, so the model
    /// "remembers" earlier messages. This fails if a turn is dropped, reordered, or mis-marked.
    #[test]
    fn chatml_preserves_multi_turn_order_and_markers() {
        let p = format_chatml(&convo());

        // Hangs at the assistant tag so the model completes the reply.
        assert!(
            p.ends_with("<|im_start|>assistant\n"),
            "prompt must end ready for the assistant to complete: {p:?}"
        );
        // Correct role markers for each turn.
        assert!(p.contains("<|im_start|>system\nYou are helpful.<|im_end|>"));
        assert!(p.contains("<|im_start|>user\nHi<|im_end|>"));
        assert!(p.contains("<|im_start|>assistant\nHello!<|im_end|>"));
        assert!(p.contains("<|im_start|>user\nWhat is 2+2?<|im_end|>"));
        // Strict conversation order (the teeth).
        let sys = p.find("You are helpful.").unwrap();
        let u1 = p.find("Hi").unwrap();
        let a1 = p.find("Hello!").unwrap();
        let u2 = p.find("What is 2+2?").unwrap();
        assert!(
            sys < u1 && u1 < a1 && a1 < u2,
            "turns must appear in conversation order (sys<u1<a1<u2), got {sys},{u1},{a1},{u2}"
        );
    }

    /// Gemma has no system role and calls the assistant "model"; system content folds into the first user
    /// turn. This fails if we leak a raw `system`/`assistant` role marker (which Gemma would emit as literal
    /// text and never stop) or drop the fold.
    #[test]
    fn gemma_folds_system_into_first_user_and_uses_model_role() {
        let p = format_gemma(&convo());

        assert!(
            p.ends_with("<start_of_turn>model\n"),
            "prompt must end at the model tag: {p:?}"
        );
        // System folded into the FIRST user turn.
        assert!(
            p.contains("<start_of_turn>user\nYou are helpful.\n\nHi<end_of_turn>"),
            "system must fold into the first user turn: {p:?}"
        );
        // Assistant rendered as "model", not "assistant".
        assert!(p.contains("<start_of_turn>model\nHello!<end_of_turn>"));
        // Later user turn is NOT re-folded with the system prefix.
        assert!(p.contains("<start_of_turn>user\nWhat is 2+2?<end_of_turn>"));
        // Gemma must never see raw system/assistant role tokens (it would echo them and never stop).
        assert!(
            !p.contains("<start_of_turn>system") && !p.contains("<start_of_turn>assistant"),
            "no raw system/assistant role markers for Gemma: {p:?}"
        );
        // Order preserved.
        let hi = p.find("Hi<end_of_turn>").unwrap();
        let hello = p.find("Hello!").unwrap();
        let q = p.find("What is 2+2?").unwrap();
        assert!(hi < hello && hello < q, "conversation order must hold");
    }

    /// The single-turn convenience must match the multi-turn ChatML formatting for `[System, User]`, so the
    /// existing AI-transform path is unchanged by the refactor.
    #[test]
    fn single_turn_chatml_matches_two_message_convo() {
        let two = vec![
            ChatTurn {
                role: Role::System,
                content: "Be terse.".to_string(),
            },
            ChatTurn {
                role: Role::User,
                content: "hello".to_string(),
            },
        ];
        assert_eq!(
            format_chatml(&two),
            "<|im_start|>system\nBe terse.<|im_end|>\n<|im_start|>user\nhello<|im_end|>\n<|im_start|>assistant\n"
        );
    }

    /// Empty system → Gemma emits just the user turn (no stray blank fold), matching the pre-refactor behavior.
    #[test]
    fn gemma_empty_system_is_just_the_user_turn() {
        let msgs = vec![
            ChatTurn {
                role: Role::System,
                content: "   ".to_string(),
            },
            ChatTurn {
                role: Role::User,
                content: "ping".to_string(),
            },
        ];
        assert_eq!(
            format_gemma(&msgs),
            "<start_of_turn>user\nping<end_of_turn>\n<start_of_turn>model\n"
        );
    }
}
