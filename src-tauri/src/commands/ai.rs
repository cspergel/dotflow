//! DotFlow — commands for the "AI transform" feature (the review overlay's Rewrite / Formal / Summarize
//! chips). Each chip asks the backend to transform the selected text with a per-action instruction, routed
//! to whichever AI backend is available: the configured cloud/Ollama post-processor first, else a local
//! offline GGUF model (only in `local-llm` builds), else a clear "no backend" error.

use tauri::AppHandle;

use crate::settings::{get_settings, write_settings};

/// Reasoning models (Qwen3.x / Qwythos, DeepSeek-R1, …) emit their chain-of-thought inside `<think>…</think>`
/// before the answer. Transforms paste their result straight back into the user's document, so any reasoning
/// must be removed first. Strips every think block (closed), and drops a trailing unclosed one entirely.
fn strip_reasoning(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(start) = rest.find("<think>") {
        out.push_str(&rest[..start]);
        let after = &rest[start + "<think>".len()..];
        match after.find("</think>") {
            Some(end) => rest = &after[end + "</think>".len()..],
            None => {
                rest = ""; // unclosed reasoning — nothing usable after it
                break;
            }
        }
    }
    out.push_str(rest);
    out.trim().to_string()
}

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

    // A transform is a quick text edit, not a reasoning task. By default we append `/no_think` so reasoning
    // models (Qwythos / Qwen3.x) skip the <think> pass and answer directly — otherwise they can burn the
    // token budget thinking and leave nothing after strip_reasoning. When the user opts into reasoning
    // (`transform_reasoning`), we omit it so the model may think (slower, sometimes better on complex text);
    // the input-scaled budget below and strip_reasoning cover that path.
    let system = if settings.transform_reasoning {
        system.to_string()
    } else {
        format!("{system}\n\n/no_think")
    };

    // Preferred backend: the configured cloud/Ollama post-process LLM.
    if crate::commands::cleanup::post_process_is_configured(app.clone()) {
        return crate::actions::ai_transform_with_llm(&settings, &system, &text)
            .await
            .map(|s| strip_reasoning(&s))
            .filter(|s| !s.is_empty())
            .ok_or_else(|| {
                "The configured AI provider returned no result — check it's running and configured \
                 correctly (or select a local model)."
                    .to_string()
            });
    }

    // Fallback backend: a local offline GGUF model. Only compiled in `local-llm` builds. Uses the
    // per-task "transform" model override when set, else the default/chat model.
    #[cfg(feature = "local-llm")]
    {
        let path = settings.model_for_task("transform");
        if !path.is_empty() {
            let model_path = std::path::PathBuf::from(&path);
            if model_path.exists() {
                let system = system.clone();
                // A rewrite/formal output is ~as long as its input; a summary is shorter. Budget ~2× the
                // input length (≈ chars/4 tokens) so a long selection isn't truncated, plus headroom for any
                // reasoning, clamped to a sane range. The generator runs an 8192-token context and caps
                // generation to the room left after the prompt, so a large value here can't overflow.
                let max_new = (text.chars().count() / 4 * 2 + 512).clamp(768, 4096);
                let out = tauri::async_runtime::spawn_blocking(move || {
                    crate::dotflow::local_llm::generate_chat(&model_path, &system, &text, max_new)
                })
                .await
                .map_err(|e| format!("local generate task failed: {e}"))?;
                // Reject an empty/whitespace-only result — otherwise Apply could clobber the user's
                // selection with nothing (asymmetric with the cloud path above).
                return out.and_then(|s| {
                    let s = strip_reasoning(&s);
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
        let path = settings.model_for_task("transform");
        if !path.is_empty() && std::path::Path::new(&path).exists() {
            return true;
        }
    }

    false
}

/// Whether AI transforms let a reasoning model think first (`/no_think` omitted). Default false.
#[tauri::command]
#[specta::specta]
pub fn get_transform_reasoning(app: AppHandle) -> bool {
    get_settings(&app).transform_reasoning
}

/// Toggle whether AI transforms allow the model to reason before answering.
#[tauri::command]
#[specta::specta]
pub fn set_transform_reasoning(app: AppHandle, enabled: bool) -> Result<(), String> {
    let mut settings = get_settings(&app);
    settings.transform_reasoning = enabled;
    write_settings(&app, settings);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::strip_reasoning;

    #[test]
    fn strips_closed_think_block_keeps_answer() {
        let s = "<think>\nThe user wants a summary. Key facts: a, b, c.\n</think>\nHere is the summary.";
        assert_eq!(strip_reasoning(s), "Here is the summary.");
    }

    #[test]
    fn drops_trailing_unclosed_think() {
        // A reasoning model cut off mid-thought must not paste its scratch-work into the document.
        let s = "Answer first.<think>still reasoning and never closed";
        assert_eq!(strip_reasoning(s), "Answer first.");
    }

    #[test]
    fn passes_through_plain_text_unchanged() {
        assert_eq!(strip_reasoning("  just an answer  "), "just an answer");
    }

    #[test]
    fn handles_multiple_think_blocks() {
        let s = "<think>a</think>Keep1 <think>b</think>Keep2";
        assert_eq!(strip_reasoning(s), "Keep1 Keep2");
    }
}
