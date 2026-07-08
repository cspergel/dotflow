//! End-to-end proof that `dotflow::local_llm::generate()` produces coherent text from a local GGUF.
//!
//! This is NOT wired into the app — it's a feature-gated harness to verify the local LLM path works.
//!
//! Build/run:
//!   cargo run --example llama_generate --features local-llm
//!
//! Model path can be overridden with the DOTFLOW_TEST_MODEL env var; it defaults to the tiny Qwen2.5
//! instruct GGUF downloaded to C:/dtfb/testmodel/ during development (kept OUTSIDE the repo).

use std::path::PathBuf;

use handy_app_lib::dotflow::local_llm;

fn main() {
    // Pull transcribe-cpp's GGML into the binary first (mirrors the app's real link order, matching
    // the llama_smoke spike) so both GGML symbol sets coexist in one binary.
    transcribe_cpp::init_logging();
    let _ = transcribe_cpp::init_backends_default();

    let model_path = std::env::var("DOTFLOW_TEST_MODEL")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("C:/dtfb/testmodel/Qwen2.5-0.5B-Instruct-Q4_K_M.gguf"));

    println!("model: {}", model_path.display());

    // Qwen2.5 uses the ChatML template.
    let user = "Rewrite this more formally: 'hey whats up'";
    let prompt = format!(
        "<|im_start|>user\n{user}<|im_end|>\n<|im_start|>assistant\n",
        user = user
    );

    println!("---- prompt ----\n{prompt}\n---- generating (max 128 new tokens) ----");

    let started = std::time::Instant::now();
    match local_llm::generate(&model_path, &prompt, 128) {
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
            eprintln!("generate() failed: {e}");
            std::process::exit(1);
        }
    }
}
