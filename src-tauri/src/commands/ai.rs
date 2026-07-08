//! DotFlow — commands for the "AI transform" feature (the review overlay's Rewrite / Formal / Summarize
//! chips). Each chip asks the backend to transform the selected text with a per-action instruction, routed
//! to whichever AI backend is available: the configured cloud/Ollama post-processor first, else a local
//! offline GGUF model (only in `local-llm` builds), else a clear "no backend" error.

use tauri::AppHandle;

use crate::settings::get_settings;

/// The per-action SYSTEM prompt. Tight, and always "output only the result" so the model doesn't wrap the
/// answer in commentary. Returns `None` for an unknown action so the caller can reject it.
fn system_prompt_for(action: &str) -> Option<&'static str> {
    match action {
        "rewrite" => Some(
            "Rewrite the user's text to be clearer and more natural. Preserve all facts and meaning. \
             Output only the rewritten text, nothing else.",
        ),
        "formal" => Some(
            "Rewrite the user's text in a more formal, professional tone. Preserve all facts and meaning. \
             Output only the rewritten text.",
        ),
        "summarize" => Some(
            "Summarize the user's text concisely. Preserve key facts. Output only the summary.",
        ),
        _ => None,
    }
}

/// Transform `text` with the given `action` (`rewrite` | `formal` | `summarize`) using the best available
/// AI backend. Preference order: the configured cloud/Ollama post-processor → a local offline GGUF model
/// (only when compiled with `local-llm` and `local_llm_model_path` points at an existing file) → an error.
/// The local generate() is CPU-bound and runs on a blocking thread so it never stalls the async runtime.
#[tauri::command]
#[specta::specta]
pub async fn ai_transform(app: AppHandle, text: String, action: String) -> Result<String, String> {
    let Some(system) = system_prompt_for(&action) else {
        return Err(format!("unknown AI action: {action}"));
    };
    if text.trim().is_empty() {
        return Err("No text to transform".to_string());
    }

    let settings = get_settings(&app);

    // Preferred backend: the configured cloud/Ollama post-process LLM.
    if crate::commands::cleanup::post_process_is_configured(app.clone()) {
        return crate::actions::ai_transform_with_llm(&settings, system, &text)
            .await
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .ok_or_else(|| {
                "The configured AI provider returned no result — check it's running and configured \
                 correctly (or select a local model)."
                    .to_string()
            });
    }

    // Fallback backend: a local offline GGUF model. Only compiled in `local-llm` builds.
    #[cfg(feature = "local-llm")]
    {
        let path = settings.local_llm_model_path.trim().to_string();
        if !path.is_empty() {
            let model_path = std::path::PathBuf::from(&path);
            if model_path.exists() {
                let system = system.to_string();
                let out = tauri::async_runtime::spawn_blocking(move || {
                    crate::dotflow::local_llm::generate_chat(&model_path, &system, &text, 256)
                })
                .await
                .map_err(|e| format!("local generate task failed: {e}"))?;
                // Reject an empty/whitespace-only result — otherwise Apply could clobber the user's
                // selection with nothing (asymmetric with the cloud path above).
                return out.and_then(|s| {
                    let s = s.trim().to_string();
                    if s.is_empty() {
                        Err("The local model returned an empty result — try again or a larger model."
                            .to_string())
                    } else {
                        Ok(s)
                    }
                });
            }
        }
    }

    Err("No AI backend configured".to_string())
}

/// Whether the AI-transform chips should be enabled: true if a cloud/Ollama post-processor is configured,
/// OR (only in `local-llm` builds) a local model path is set and the file exists.
#[tauri::command]
#[specta::specta]
pub fn ai_transform_available(app: AppHandle) -> bool {
    if crate::commands::cleanup::post_process_is_configured(app.clone()) {
        return true;
    }

    #[cfg(feature = "local-llm")]
    {
        let settings = get_settings(&app);
        let path = settings.local_llm_model_path.trim();
        if !path.is_empty() && std::path::Path::new(path).exists() {
            return true;
        }
    }

    false
}
