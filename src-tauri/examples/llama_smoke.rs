//! De-risking spike smoke test for `llama-cpp-2`.
//!
//! Purpose: force the linker to resolve real llama.cpp symbols so we can prove
//! the crate compiles + links alongside transcribe-cpp's bundled GGML on this
//! Windows machine. This does NOT load a model and is NOT wired into the app.
//!
//! Build: `cargo build --features local-llm --example llama_smoke`

use llama_cpp_2::llama_backend::LlamaBackend;
use llama_cpp_2::model::params::LlamaModelParams;

fn main() {
    // CRITICAL: reference transcribe-cpp FIRST so whisper's bundled GGML
    // (ggml_init / ggml_backend_reg_* etc.) is pulled into THIS binary. Then
    // reference llama-cpp-2, which bundles its OWN GGML. Both GGML symbol sets
    // must therefore resolve in a single link unit — this is the duplicate-symbol
    // coexistence test the spike exists to answer.
    transcribe_cpp::init_logging();
    let _ = transcribe_cpp::init_backends_default();
    let whisper_devices = transcribe_cpp::devices();
    println!(
        "transcribe-cpp: GGML backends initialized; {} device(s) enumerated",
        whisper_devices.len()
    );

    // Initializing the llama backend pulls in real llama.cpp C/C++ symbols
    // (ggml_backend_*, llama_backend_init, etc.) so the linker must resolve them.
    let backend = LlamaBackend::init().expect("failed to init llama backend");

    // Touch a params struct so more symbols are referenced.
    let params = LlamaModelParams::default();
    println!(
        "llama-cpp-2 smoke: backend initialized OK; default n_gpu_layers = {}",
        params.n_gpu_layers()
    );

    drop(backend);
    println!("llama_smoke: BOTH GGMLs (transcribe-cpp + llama-cpp-2) linked into one binary OK");
}
