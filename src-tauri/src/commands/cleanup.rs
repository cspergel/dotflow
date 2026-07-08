//! DotFlow — commands for the text-cleanup feature (the "Cleanup" settings section).

use tauri::{AppHandle, Manager};

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

/// Analyze `text` and return Harper's reviewable suggestions (spans + replacements) WITHOUT changing it —
/// the data source for the Grammarly-style review panel, where the user accepts/rejects each fix. Offline.
///
/// Runs the Harper analysis on a blocking thread (`spawn_blocking`) rather than inline: as a synchronous
/// command it ran on the main thread and froze the whole review card (chips, drag, Apply) for the couple
/// of seconds Harper takes on a large selection. Async + spawn_blocking keeps the UI responsive.
#[tauri::command]
#[specta::specta]
pub async fn analyze_text(text: String) -> Vec<crate::dotflow::grammar::TextSuggestion> {
    tauri::async_runtime::spawn_blocking(move || crate::dotflow::grammar::analyze(&text))
        .await
        .unwrap_or_default()
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

/// Paste a reviewed result back into the field the review hotkey was fired from. Refocuses the saved
/// window; ONLY pastes if the refocus actually succeeded (else the result would land in the wrong app);
/// then restores the user's original clipboard. Hides the overlay first (synchronously).
#[tauri::command]
#[specta::specta]
pub async fn apply_review_result(app: AppHandle, text: String) -> Result<(), String> {
    use tauri_plugin_clipboard_manager::ClipboardExt;

    crate::overlay::hide_review_overlay(&app); // [F13] synchronous hide — card gone before we paste

    // Take the whole context (hwnd + original clipboard), clearing it so a stray second apply no-ops.
    let ctx = app
        .try_state::<crate::ReviewContext>()
        .and_then(|s| s.0.lock().ok().and_then(|mut c| c.take())); // [F12] .ok(), no unwrap

    let app_c = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let hwnd = ctx.as_ref().and_then(|c| c.source_hwnd);
        // [F3] refocus, then GUARD: only paste if the source window is real AND actually foreground.
        let mut refocused = false;
        if let Some(hwnd) = hwnd {
            if crate::input::is_window(hwnd) {
                crate::input::force_foreground(hwnd);
                for _ in 0..25 {
                    // poll up to ~500ms
                    if crate::input::get_foreground_window() == Some(hwnd) {
                        refocused = true;
                        break;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(20));
                }
            }
        }
        log::info!("apply_review_result: refocused={refocused} hwnd={hwnd:?}"); // focus checkpoint

        if refocused && !text.trim().is_empty() {
            if let Err(e) = crate::clipboard::inject_bulk(&text, &app_c) {
                log::warn!("apply_review_result: paste failed: {e}");
            }
        } else if !refocused {
            // [F3] Do NOT blind-paste into the wrong window. If there's a real result, leave it on the
            // clipboard so the user can paste it manually and tell them, then skip the restore below.
            // If the result is EMPTY, don't blank the clipboard — fall through to restore the ORIGINAL.
            if !text.trim().is_empty() {
                let _ = app_c.clipboard().write_text(&text);
                log::warn!(
                    "apply_review_result: could not refocus source — result left on clipboard for manual paste"
                );
                return; // skip the original-clipboard restore below: the result IS the clipboard now
            }
        }

        // [F1] restore the user's ORIGINAL clipboard (inject_bulk left the result/selection on it).
        if let Some(c) = ctx {
            let _ = app_c.clipboard().write_text(&c.original_clipboard);
        }
    })
    .await
    .map_err(|e| format!("apply task failed: {e}"))?;
    Ok(())
}

/// Cancel/close the review card without pasting. [F1] Restores the user's ORIGINAL clipboard, because the
/// copy phase left the SELECTED text on the clipboard — cancel must put back what the user had.
#[tauri::command]
#[specta::specta]
pub fn cancel_review(app: AppHandle) {
    use tauri_plugin_clipboard_manager::ClipboardExt;

    crate::overlay::hide_review_overlay(&app); // clears REVIEW_OPEN
    if let Some(ctx) = app
        .try_state::<crate::ReviewContext>()
        .and_then(|s| s.0.lock().ok().and_then(|mut c| c.take()))
    {
        let _ = app.clipboard().write_text(&ctx.original_clipboard); // [F1]
    }
}

/// [F11] Return the pending review payload `(selected_text, ai_available)` so a late-mounting overlay can
/// PULL its text on mount if it missed the `review-text` emit. READS the context (does NOT take/clear it —
/// apply/cancel still need it later).
#[tauri::command]
#[specta::specta]
pub fn get_pending_review(app: AppHandle) -> Option<(String, bool)> {
    app.try_state::<crate::ReviewContext>().and_then(|s| {
        s.0.lock()
            .ok()
            .and_then(|c| c.as_ref().and_then(|x| x.payload.clone()))
    })
}
