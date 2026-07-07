//! DotFlow — commands for the text-cleanup feature (the "Cleanup" settings section).

use tauri::AppHandle;

use crate::settings::get_settings;

/// Run the shared cleanup pipeline over `text` and return the cleaned result — the same path the
/// Ctrl+Shift+U hotkey uses (post-process LLM if configured, else offline Harper grammar + deterministic
/// tidy). Powers the in-app "Try it" box so cleanup can be used and verified without the global hotkey.
#[tauri::command]
#[specta::specta]
pub async fn preview_cleanup(app: AppHandle, text: String) -> Result<String, String> {
    log::info!("preview_cleanup called ({} chars)", text.len());
    let settings = get_settings(&app);
    let out = crate::actions::resolve_cleanup(&settings, &text).await;
    log::info!("preview_cleanup done ({} chars out)", out.len());
    Ok(out)
}

/// Whether a post-process LLM is fully configured (provider + model + prompt). The Cleanup section uses this
/// to show whether the hotkey is using the AI tier or the offline Harper tier.
#[tauri::command]
#[specta::specta]
pub fn post_process_is_configured(app: AppHandle) -> bool {
    let settings = get_settings(&app);
    let Some(provider) = settings.active_post_process_provider() else {
        return false;
    };
    let has_model = settings
        .post_process_models
        .get(&provider.id)
        .is_some_and(|m| !m.trim().is_empty());
    let has_prompt = settings
        .post_process_selected_prompt_id
        .as_ref()
        .is_some_and(|id| settings.post_process_prompts.iter().any(|p| &p.id == id));
    has_model && has_prompt
}
