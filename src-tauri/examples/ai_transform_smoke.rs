//! End-to-end proof that `dotflow::local_llm::generate_chat()` — the chat-templated path behind the
//! review overlay's AI-transform chips — produces coherent text from a local GGUF.
//!
//! Unlike `llama_generate` (which hand-wraps a ChatML prompt), this exercises the real code path
//! `ai_transform` uses: system + user messages fed through the model's built-in chat template (ChatML
//! fallback). NOT wired into the app; a feature-gated verification harness.
//!
//! Build/run:
//!   cargo run --example ai_transform_smoke --features local-llm
//!
//! Model path can be overridden with DOTFLOW_TEST_MODEL; defaults to the Qwen2.5-0.5B instruct GGUF
//! kept OUTSIDE the repo at C:/dtfb/testmodel/.

use std::path::PathBuf;

use handy_app_lib::dotflow::local_llm;

fn main() {
    transcribe_cpp::init_logging();
    let _ = transcribe_cpp::init_backends_default();

    let model_path = std::env::var("DOTFLOW_TEST_MODEL")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("C:/dtfb/testmodel/Qwen2.5-0.5B-Instruct-Q4_K_M.gguf"));

    println!("model: {}", model_path.display());

    // Mirrors the "formal" AI-transform action.
    let system =
        "Rewrite the user's text in a more formal, professional tone. Preserve all facts and \
                  meaning. Output only the rewritten text.";
    let user = "hey, just wanted to let you know the thing is done, lmk if you need anything else";

    println!("---- system ----\n{system}\n---- user ----\n{user}\n---- generating (max 256) ----");

    let started = std::time::Instant::now();
    match local_llm::generate_chat(&model_path, system, user, 256) {
        Ok(text) => {
            println!("---- OUTPUT ({} ms) ----", started.elapsed().as_millis());
            println!("{text}");
            println!("---- END OUTPUT ----");
            if text.trim().is_empty() {
                eprintln!("WARNING: generation returned empty text");
                std::process::exit(2);
            }
        }
        Err(e) => {
            eprintln!("generate_chat() failed: {e}");
            std::process::exit(1);
        }
    }
}
