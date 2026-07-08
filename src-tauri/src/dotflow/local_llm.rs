//! Local LLM text generation for DotFlow (feature-gated behind `local-llm`).
//!
//! This is the effectful, offline text-generation shell that DotFlow's selection→review overlay
//! will call to rewrite/clean up a highlighted passage using a small local instruct model. It wraps
//! the `llama-cpp-2` crate (v0.1.139) — a thin, *unsafe* binding over llama.cpp — behind a single
//! total-ish function that never panics into the caller: any llama.cpp panic is caught and mapped to
//! `Err(String)` so a bad model / OOM / assert cannot take down the Tauri app.
//!
//! Everything here is CPU-only (`n_gpu_layers = 0`) so it coexists with transcribe-cpp's GPU whisper
//! backend without fighting over the GPU. The llama backend is initialized exactly once, process-wide,
//! via `OnceLock` — llama.cpp's `llama_backend_init` must not run twice.
//!
//! API shape used (llama-cpp-2 0.1.139):
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
use std::path::Path;
use std::sync::OnceLock;

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

/// Build the prompt string for a system+user chat turn. Prefers the template baked into the GGUF (so
/// Qwen, Gemma, Llama, … each get their own correct markers); falls back to ChatML — which suits the
/// Qwen2.5 test model — when the model ships no template or applying it fails. Returns the prompt plus
/// the `AddBos` policy: the built-in template owns any BOS (so `Never`), while the bare ChatML fallback
/// defers to the tokenizer config (`Always`), matching [`generate`].
fn build_chat_prompt(model: &LlamaModel, system: &str, user: &str) -> (String, AddBos) {
    let messages = [
        LlamaChatMessage::new("system".to_string(), system.to_string()),
        LlamaChatMessage::new("user".to_string(), user.to_string()),
    ];
    if let (Ok(m0), Ok(m1)) = (&messages[0], &messages[1]) {
        if let Ok(tmpl) = model.chat_template(None) {
            // add_ass=true leaves the prompt hanging at the assistant tag so the model completes the reply.
            if let Ok(rendered) = model.apply_chat_template(&tmpl, &[m0.clone(), m1.clone()], true)
            {
                return (rendered, AddBos::Never);
            }
        }
    }
    // ChatML fallback (Qwen-style). parse_special=true in str_to_token turns these markers into real
    // special tokens, so this is a valid prompt even though it's assembled as plain text.
    (
        format!(
            "<|im_start|>system\n{system}<|im_end|>\n<|im_start|>user\n{user}<|im_end|>\n<|im_start|>assistant\n"
        ),
        AddBos::Always,
    )
}

fn generate_chat_inner(
    model_path: &Path,
    system: &str,
    user: &str,
    max_new_tokens: usize,
) -> Result<String, String> {
    if !model_path.exists() {
        return Err(format!(
            "model file does not exist: {}",
            model_path.display()
        ));
    }

    let backend = backend()?;
    let model_params = LlamaModelParams::default().with_n_gpu_layers(0);
    let model = LlamaModel::load_from_file(backend, model_path, &model_params)
        .map_err(|e| format!("failed to load model: {e}"))?;

    let (prompt, add_bos) = build_chat_prompt(&model, system, user);
    run_generation(backend, &model, &prompt, add_bos, max_new_tokens)
}

fn generate_inner(
    model_path: &Path,
    prompt: &str,
    max_new_tokens: usize,
) -> Result<String, String> {
    if !model_path.exists() {
        return Err(format!(
            "model file does not exist: {}",
            model_path.display()
        ));
    }

    let backend = backend()?;

    // CPU only: keep every layer on the CPU so we don't contend with the GPU whisper backend.
    let model_params = LlamaModelParams::default().with_n_gpu_layers(0);
    let model = LlamaModel::load_from_file(backend, model_path, &model_params)
        .map_err(|e| format!("failed to load model: {e}"))?;

    run_generation(backend, &model, prompt, AddBos::Always, max_new_tokens)
}

/// Create a context, tokenize `prompt`, and greedily decode up to `max_new_tokens`. Shared by the plain
/// [`generate`] path and the chat-templated [`generate_chat`] path. `add_bos` lets the chat path suppress
/// a BOS the template already emitted while the plain path keeps the tokenizer-config default.
fn run_generation(
    backend: &LlamaBackend,
    model: &LlamaModel,
    prompt: &str,
    add_bos: AddBos,
    max_new_tokens: usize,
) -> Result<String, String> {
    // A 4096-token context is plenty for a "rewrite this selection" round-trip. Cap it at what the
    // model was actually trained on so we never ask for more than the weights support.
    let n_ctx = 4096u32.min(model.n_ctx_train().max(1));
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

    for _ in 0..max_new_tokens {
        // Sample from the logits of the last token in the current batch.
        let token = sampler.sample(&ctx, batch.n_tokens() - 1);
        sampler.accept(token);

        if model.is_eog_token(token) {
            break;
        }

        // Detokenize as plaintext so any stray control/special token renders harmlessly rather than as
        // literal marker text.
        match model.token_to_str(token, Special::Plaintext) {
            Ok(piece) => output.push_str(&piece),
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
